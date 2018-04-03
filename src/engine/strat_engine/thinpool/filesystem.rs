// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::{Path, PathBuf};

use devicemapper::{Bytes, DmDevice, DmName, DmUuid, IEC, SECTOR_SIZE, Sectors, ThinDev, ThinDevId,
                   ThinPoolDev, ThinStatus};

use mnt::{MountIter, MountParam};
use nix::mount::{MsFlags, mount, umount};
use nix::sys::statvfs::statvfs;
use tempdir::TempDir;

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::engine::Filesystem;
use super::super::super::types::{FilesystemUuid, Name};

use super::super::dm::get_dm;
use super::super::serde_structs::FilesystemSave;

use super::util::{create_fs, set_uuid, xfs_growfs};

/// TODO: confirm that 256 MiB leaves enough time for stratisd to respond and extend before
/// the filesystem is out of space.
pub const FILESYSTEM_LOWATER: Sectors = Sectors(256 * IEC::Mi / (SECTOR_SIZE as u64)); // = 256 MiB

#[derive(Debug)]
pub struct StratFilesystem {
    thin_dev: ThinDev,
}

pub enum FilesystemStatus {
    Good,
    XfsGrowFailed,
    ThinDevExtendFailed,
    Failed,
}

impl StratFilesystem {
    /// Create a StratFilesystem on top of the given ThinDev.
    pub fn initialize(fs_id: FilesystemUuid, thin_dev: ThinDev) -> StratisResult<StratFilesystem> {
        let fs = StratFilesystem::setup(thin_dev);

        create_fs(&fs.devnode(), fs_id)?;
        Ok(fs)
    }

    /// Build a StratFilesystem that includes the ThinDev and related info.
    pub fn setup(thin_dev: ThinDev) -> StratFilesystem {
        StratFilesystem { thin_dev }
    }

    /// Create a snapshot of the filesystem. Return the resulting filesystem/ThinDev
    /// to the caller.  Use snapshot_name for the Stratis filesystem name.  Use
    /// snapshot_dmname for the new name of the ThinDev allocated for the snapshot.
    /// Mounting a filesystem with a duplicate UUID would require special handling,
    /// so snapshot_fs_uuid is used to update the new snapshot filesystem so it has
    /// a unique UUID.
    #[allow(too_many_arguments)]
    pub fn snapshot(&self,
                    thin_pool: &ThinPoolDev,
                    snapshot_name: &str,
                    snapshot_dm_name: &DmName,
                    snapshot_dm_uuid: Option<&DmUuid>,
                    snapshot_fs_name: &Name,
                    snapshot_fs_uuid: FilesystemUuid,
                    snapshot_thin_id: ThinDevId)
                    -> StratisResult<StratFilesystem> {

        match self.thin_dev
                  .snapshot(get_dm(),
                            snapshot_dm_name,
                            snapshot_dm_uuid,
                            thin_pool,
                            snapshot_thin_id) {
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

                // FIXME: get_mount_point doesn't work so assume we need to mount/unmount
                let tmp_dir = TempDir::new("stratis_mp_")?;
                // Mount the snapshot with the "nouuid" option. mount
                // will fail due to duplicate UUID otherwise.
                mount(Some(&thin_dev.devnode()),
                      tmp_dir.path(),
                      Some("xfs"),
                      MsFlags::empty(),
                      Some("nouuid"))?;
                umount(tmp_dir.path())?;

                set_uuid(&thin_dev.devnode(), snapshot_fs_uuid)?;
                Ok(StratFilesystem::setup(thin_dev))
            }
            Err(e) => {
                Err(StratisError::Engine(ErrorEnum::Error,
                                         format!("failed to create {} snapshot for {} - {}",
                                                 snapshot_name,
                                                 snapshot_fs_name,
                                                 e)))
            }
        }
    }

    /// check if filesystem is getting full and needs to be extended
    /// TODO: deal with the thindev in a Fail state.
    pub fn check(&mut self) -> StratisResult<FilesystemStatus> {
        match self.thin_dev.status(get_dm())? {
            ThinStatus::Working(_) => {
                if let Some(mount_point) = self.get_mount_point()? {
                    let (fs_total_bytes, fs_total_used_bytes) = fs_usage(&mount_point)?;
                    let free_bytes = fs_total_bytes - fs_total_used_bytes;
                    if free_bytes.sectors() < FILESYSTEM_LOWATER {
                        let mut table = self.thin_dev.table().table.clone();
                        table.length = self.thin_dev.size() +
                                       self.extend_size(self.thin_dev.size());
                        if self.thin_dev.set_table(get_dm(), table).is_err() {
                            return Ok(FilesystemStatus::ThinDevExtendFailed);
                        }
                        if xfs_growfs(&mount_point).is_err() {
                            return Ok(FilesystemStatus::XfsGrowFailed);
                        }
                    }
                }
                // TODO: do anything when filesystem is not mounted?
                // TODO: periodically kick off fstrim?
            }
            ThinStatus::Fail => return Ok(FilesystemStatus::Failed),
        }
        Ok(FilesystemStatus::Good)
    }

    /// The thin id for the thin device that backs this filesystem.
    #[cfg(test)]
    pub fn thin_id(&self) -> ThinDevId {
        self.thin_dev.id()
    }

    /// Return an extend size for the thindev under the filesystem
    /// TODO: returning the current size will double the space provisioned to
    /// the thin device.  We should determine if this is a reasonable value.
    fn extend_size(&self, current_size: Sectors) -> Sectors {
        current_size
    }

    /// Get one (non-deterministic in the presence of errors) of the mount_point(s) for the file
    /// system that is contained on the block device referred to as self.devnode(), i.e. the device
    /// node, while ignoring parse errors as long as at least one mount point is found.
    pub fn get_mount_point(&self) -> StratisResult<Option<PathBuf>> {
        let device_node = self.devnode();
        let search = device_node.to_str().ok_or_else(|| StratisError::Engine(ErrorEnum::Error,
                                    format!("Unable to represent devnode as string {:?}", *self)))?;

        let m_iter = MountIter::new_from_proc()
            .map_err(|e| {
                         StratisError::Engine(ErrorEnum::Error,
                                              format!("Error reading /proc/mounts {:?}", e))
                     })?;

        let mut last_error: Option<String> = None;
        for mp in m_iter {
            match mp {
                Ok(mount) => {
                    if mount.contains(&MountParam::Spec(search)) {
                        return Ok(Some(mount.file));
                    }
                }
                Err(e) => {
                    last_error = Some(format!("Error during parsing {:?} {:?}", *self, e));
                }
            }
        }

        last_error.map_or(Ok(None), |e| Err(StratisError::Engine(ErrorEnum::Error, e)))
    }

    /// Tear down the filesystem.
    pub fn teardown(self) -> StratisResult<()> {
        self.thin_dev.teardown(get_dm())?;
        Ok(())
    }

    /// Destroy the filesystem.
    pub fn destroy(self, thin_pool: &ThinPoolDev) -> StratisResult<()> {
        self.thin_dev.destroy(get_dm(), thin_pool)?;
        Ok(())
    }

    pub fn record(&self, name: &Name, uuid: FilesystemUuid) -> FilesystemSave {
        FilesystemSave {
            name: name.to_owned(),
            uuid,
            thin_id: self.thin_dev.id(),
            size: self.thin_dev.size(),
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
}

impl Filesystem for StratFilesystem {
    fn devnode(&self) -> PathBuf {
        self.thin_dev.devnode()
    }
}

/// Return total bytes allocated to the filesystem, total bytes used by data/metadata
pub fn fs_usage(mount_point: &Path) -> StratisResult<(Bytes, Bytes)> {
    let stat = statvfs(mount_point)?;

    // Upcast all arch-dependent variable width values to u64
    let (block_size, blocks, blocks_free) =
        (stat.block_size() as u64, stat.blocks() as u64, stat.blocks_free() as u64);
    Ok((Bytes(block_size * blocks), Bytes(block_size * (blocks - blocks_free))))
}
