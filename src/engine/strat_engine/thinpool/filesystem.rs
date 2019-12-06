// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
};

use devicemapper::{
    Bytes, DmDevice, DmName, DmUuid, Sectors, ThinDev, ThinDevId, ThinPoolDev, ThinStatus, IEC,
};

use libmount;
use nix::{
    mount::{mount, umount, MsFlags},
    sys::statvfs::statvfs,
};
use tempfile;

use crate::{
    engine::{
        engine::Filesystem,
        strat_engine::{
            cmd::{create_fs, set_uuid, udev_settle, xfs_growfs},
            devlinks,
            dm::get_dm,
            names::{format_thin_ids, ThinRole},
            serde_structs::FilesystemSave,
            thinpool::{thinpool::DATA_LOWATER, DATA_BLOCK_SIZE},
        },
        types::{FilesystemUuid, MaybeDbusPath, Name, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB

const TEMP_MNT_POINT_PREFIX: &str = "stratis_mp_";

/// Set the low water mark on the filesystem at 4 times the data low water.  The filesystem
/// expansion check is triggered by crossing the data low water mark for the thin pool.
pub const FILESYSTEM_LOWATER: Sectors = Sectors(4 * (DATA_LOWATER.0 * DATA_BLOCK_SIZE.0));

#[derive(Debug)]
pub struct StratFilesystem {
    thin_dev: ThinDev,
    created: DateTime<Utc>,
    dbus_path: MaybeDbusPath,
}

pub enum FilesystemStatus {
    Good,
    XfsGrowFailed,
    ThinDevExtendFailed,
    Failed,
}

impl StratFilesystem {
    /// Create a StratFilesystem on top of the given ThinDev.
    pub fn initialize(
        pool_uuid: PoolUuid,
        thinpool_dev: &ThinPoolDev,
        size: Option<Sectors>,
        id: ThinDevId,
    ) -> StratisResult<(FilesystemUuid, StratFilesystem)> {
        let fs_uuid = Uuid::new_v4();
        let (dm_name, dm_uuid) = format_thin_ids(pool_uuid, ThinRole::Filesystem(fs_uuid));
        let mut thin_dev = ThinDev::new(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            size.unwrap_or(DEFAULT_THIN_DEV_SIZE),
            thinpool_dev,
            id,
        )?;

        if let Err(err) = create_fs(&thin_dev.devnode(), Some(fs_uuid)) {
            udev_settle().unwrap_or_else(|err| {
                warn!("{}", err);
                sleep(Duration::from_secs(5));
            });
            if let Err(err2) = thin_dev.destroy(get_dm(), thinpool_dev) {
                error!(
                    "While handling create_fs error, thin_dev.destroy() failed: {}",
                    err2
                );
                // This will result in a dangling DM device that will prevent
                // the thinpool from being destroyed, and wasted space in the
                // thinpool.
                // TODO: Recover. But how?
            }
            return Err(err);
        }

        Ok((
            fs_uuid,
            StratFilesystem {
                thin_dev,
                created: Utc::now(),
                dbus_path: MaybeDbusPath(None),
            },
        ))
    }

    /// Build a StratFilesystem that includes the ThinDev and related info.
    pub fn setup(
        pool_uuid: PoolUuid,
        thinpool_dev: &ThinPoolDev,
        fssave: &FilesystemSave,
    ) -> StratisResult<StratFilesystem> {
        let (dm_name, dm_uuid) = format_thin_ids(pool_uuid, ThinRole::Filesystem(fssave.uuid));
        let thin_dev = ThinDev::setup(
            get_dm(),
            &dm_name,
            Some(&dm_uuid),
            fssave.size,
            thinpool_dev,
            fssave.thin_id,
        )?;
        Ok(StratFilesystem {
            thin_dev,
            created: Utc.timestamp(fssave.created as i64, 0),
            dbus_path: MaybeDbusPath(None),
        })
    }

    /// Create a snapshot of the filesystem. Return the resulting filesystem/ThinDev
    /// to the caller.  Use snapshot_name for the Stratis filesystem name.  Use
    /// snapshot_dmname for the new name of the ThinDev allocated for the snapshot.
    /// Mounting a filesystem with a duplicate UUID would require special handling,
    /// so snapshot_fs_uuid is used to update the new snapshot filesystem so it has
    /// a unique UUID.
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot(
        &self,
        thin_pool: &ThinPoolDev,
        snapshot_name: &str,
        snapshot_dm_name: &DmName,
        snapshot_dm_uuid: Option<&DmUuid>,
        snapshot_fs_name: &Name,
        snapshot_fs_uuid: FilesystemUuid,
        snapshot_thin_id: ThinDevId,
    ) -> StratisResult<StratFilesystem> {
        match self.thin_dev.snapshot(
            get_dm(),
            snapshot_dm_name,
            snapshot_dm_uuid,
            thin_pool,
            snapshot_thin_id,
        ) {
            Ok(thin_dev) => {
                // If the source is mounted, XFS puts a dummy record in the
                // log to enforce replay of the snapshot to deal with any
                // orphaned inodes. The dummy record put the log in a dirty
                // state. xfs_admin won't allow a filesystem UUID
                // to be updated when the log is dirty.  To clear the log
                // we mount/unmount the filesystem before updating the UUID.
                //
                // If the source is unmounted the XFS log will be clean so
                // we can skip the mount/unmount.
                if !self.mount_points()?.is_empty() {
                    let tmp_dir = tempfile::Builder::new()
                        .prefix(TEMP_MNT_POINT_PREFIX)
                        .tempdir()?;
                    // Mount the snapshot with the "nouuid" option. mount
                    // will fail due to duplicate UUID otherwise.
                    mount(
                        Some(&thin_dev.devnode()),
                        tmp_dir.path(),
                        Some("xfs"),
                        MsFlags::empty(),
                        Some("nouuid"),
                    )?;
                    umount(tmp_dir.path())?;
                }

                set_uuid(&thin_dev.devnode(), snapshot_fs_uuid)?;
                Ok(StratFilesystem {
                    thin_dev,
                    created: Utc::now(),
                    dbus_path: MaybeDbusPath(None),
                })
            }
            Err(e) => Err(StratisError::Engine(
                ErrorEnum::Error,
                format!(
                    "failed to create {} snapshot for {} - {}",
                    snapshot_name, snapshot_fs_name, e
                ),
            )),
        }
    }

    /// check if filesystem is getting full and needs to be extended
    /// TODO: deal with the thindev in a Fail state.
    pub fn check(&mut self) -> StratisResult<(FilesystemStatus, bool)> {
        match self.thin_dev.status(get_dm())? {
            ThinStatus::Working(_) => {
                if let Some(mount_point) = self.mount_points()?.first() {
                    let (fs_total_bytes, fs_total_used_bytes) = fs_usage(mount_point)?;
                    let free_bytes = fs_total_bytes - fs_total_used_bytes;
                    if free_bytes.sectors() < FILESYSTEM_LOWATER {
                        let mut table = self.thin_dev.table().table.clone();
                        table.length =
                            self.thin_dev.size() + self.extend_size(self.thin_dev.size());
                        if self.thin_dev.set_table(get_dm(), table).is_err() {
                            return Ok((FilesystemStatus::ThinDevExtendFailed, false));
                        }
                        if xfs_growfs(mount_point).is_err() {
                            return Ok((FilesystemStatus::XfsGrowFailed, true));
                        }
                        return Ok((FilesystemStatus::Good, true));
                    }
                }
                // TODO: do anything when filesystem is not mounted?
                // TODO: periodically kick off fstrim?
                Ok((FilesystemStatus::Good, false))
            }
            ThinStatus::Error => {
                let error_msg = format!(
                    "Unable to get status for filesystem thin device {}",
                    self.thin_dev.device()
                );
                Err(StratisError::Engine(ErrorEnum::Error, error_msg))
            }
            ThinStatus::Fail => Ok((FilesystemStatus::Failed, false)),
        }
    }

    /// Return an extend size for the thindev under the filesystem
    /// TODO: returning the current size will double the space provisioned to
    /// the thin device.  We should determine if this is a reasonable value.
    fn extend_size(&self, current_size: Sectors) -> Sectors {
        current_size
    }

    /// Tear down the filesystem.
    pub fn teardown(&mut self) -> StratisResult<()> {
        self.thin_dev.teardown(get_dm())?;
        Ok(())
    }

    /// Destroy the filesystem.
    pub fn destroy(&mut self, thin_pool: &ThinPoolDev) -> StratisResult<()> {
        self.thin_dev.destroy(get_dm(), thin_pool)?;
        Ok(())
    }

    pub fn record(&self, name: &Name, uuid: FilesystemUuid) -> FilesystemSave {
        FilesystemSave {
            name: name.to_owned(),
            uuid,
            thin_id: self.thin_dev.id(),
            size: self.thin_dev.size(),
            created: self.created.timestamp() as u64,
        }
    }

    pub fn suspend(&mut self, flush: bool) -> StratisResult<()> {
        self.thin_dev.suspend(get_dm(), flush)?;
        Ok(())
    }

    pub fn resume(&mut self) -> StratisResult<()> {
        self.thin_dev.resume(get_dm())?;
        Ok(())
    }

    /// Find places where this filesystem is mounted.
    fn mount_points(&self) -> StratisResult<Vec<PathBuf>> {
        // Use major:minor values to find mounts for this filesystem
        let major = u64::from(self.thin_dev.device().major);
        let minor = u64::from(self.thin_dev.device().minor);

        let mut mount_data = String::new();
        File::open("/proc/self/mountinfo")?.read_to_string(&mut mount_data)?;
        let parser = libmount::mountinfo::Parser::new(mount_data.as_bytes());

        let mut ret_vec = Vec::new();
        for mp in parser {
            match mp {
                Ok(mount) => {
                    if mount.major as u64 == major && mount.minor as u64 == minor {
                        ret_vec.push(PathBuf::from(&mount.mount_point));
                    }
                }
                Err(e) => {
                    let error_msg = format!("Error during parsing {:?}: {:?}", *self, e);
                    return Err(StratisError::Engine(ErrorEnum::Error, error_msg));
                }
            }
        }

        Ok(ret_vec)
    }
    #[cfg(test)]
    pub fn thindev_size(&self) -> Sectors {
        self.thin_dev.size()
    }
}

impl Filesystem for StratFilesystem {
    fn devnode(&self) -> PathBuf {
        self.thin_dev.devnode()
    }

    fn created(&self) -> DateTime<Utc> {
        self.created
    }

    fn path_to_mount_filesystem(&self, pool_name: &str, fs_name: &str) -> PathBuf {
        devlinks::filesystem_mount_path(pool_name, fs_name)
    }

    fn used(&self) -> StratisResult<Bytes> {
        match self.thin_dev.status(get_dm())? {
            ThinStatus::Working(wk_status) => Ok(wk_status.nr_mapped_sectors.bytes()),
            ThinStatus::Error => {
                let error_msg = format!(
                    "Unable to get status for filesystem thin device {}",
                    self.thin_dev.device()
                );
                Err(StratisError::Engine(ErrorEnum::Error, error_msg))
            }
            ThinStatus::Fail => {
                let error_msg = format!("ThinDev {} is in a failed state", self.thin_dev.device());
                Err(StratisError::Engine(ErrorEnum::Error, error_msg))
            }
        }
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }
}

/// Return total bytes allocated to the filesystem, total bytes used by data/metadata
pub fn fs_usage(mount_point: &Path) -> StratisResult<(Bytes, Bytes)> {
    let stat = statvfs(mount_point)?;

    // Upcast all arch-dependent variable width values to u64
    let (block_size, blocks, blocks_free) = (
        stat.block_size() as u64,
        stat.blocks() as u64,
        stat.blocks_free() as u64,
    );
    Ok((
        Bytes(block_size * blocks),
        Bytes(block_size * (blocks - blocks_free)),
    ))
}
