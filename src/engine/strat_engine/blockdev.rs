// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use time::Timespec;
use devicemapper::Device;
use uuid::Uuid;

use types::{Bytes, Sectors};
use engine::{DevUuid, EngineResult, EngineError, ErrorEnum, PoolUuid};

use consts::IEC;
use super::device::blkdev_size;
use super::metadata::{StaticHeader, BDA, validate_mda_size};
use super::engine::DevOwnership;
pub use super::BlockDevSave;

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi as u64);


/// Find all Stratis Blockdevs.
///
/// Returns a map of pool uuids to maps of blockdev uuids to blockdevs.
pub fn find_all() -> EngineResult<HashMap<PoolUuid, HashMap<DevUuid, BlockDev>>> {

    /// If a Path refers to a valid Stratis blockdev, return a BlockDev
    /// struct. Otherwise, return None. Return an error if there was
    /// a problem inspecting the device.
    fn setup(devnode: &Path) -> EngineResult<Option<BlockDev>> {
        let mut f = try!(OpenOptions::new()
            .read(true)
            .open(devnode));

        if let Some(bda) = BDA::load(&mut f).ok() {
            let dev = try!(Device::from_str(&devnode.to_string_lossy()));
            Ok(Some(BlockDev {
                dev: dev,
                devnode: devnode.to_owned(),
                bda: bda,
            }))
        } else {
            Ok(None)
        }
    }

    let mut pool_map = HashMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let devnode = match dir_e {
            Ok(d) => d.path(),
            Err(_) => continue,
        };

        match setup(&devnode) {
            Ok(Some(blockdev)) => {
                pool_map.entry(blockdev.pool_uuid().clone())
                    .or_insert_with(HashMap::new)
                    .insert(blockdev.uuid().clone(), blockdev);
            }
            _ => continue,
        };
    }

    Ok(pool_map)
}



/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
// FIXME: BTreeSet -> HashSet once Device is hashable
pub fn initialize(pool_uuid: &PoolUuid,
                  devices: BTreeSet<Device>,
                  mda_size: Sectors,
                  force: bool)
                  -> EngineResult<Vec<BlockDev>> {
    /// Gets device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the blockdev's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    fn dev_info(dev: &Device) -> EngineResult<(PathBuf, Bytes, DevOwnership, File)> {
        let devnode = try!(dev.path().ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidInput,
                           format!("could not get device node from dev {}", dev.dstr()))
        }));
        let mut f = try!(OpenOptions::new()
            .read(true)
            .write(true)
            .open(&devnode)
            .map_err(|_| {
                io::Error::new(ErrorKind::PermissionDenied,
                               format!("Could not open {}", devnode.display()))
            }));

        let dev_size = try!(blkdev_size(&f));

        let ownership = match StaticHeader::determine_ownership(&mut f) {
            Ok(ownership) => ownership,
            Err(err) => {
                let error_message = format!("{} for device {}", err, devnode.display());
                return Err(EngineError::Engine(ErrorEnum::Invalid, error_message));
            }
        };

        Ok((devnode, dev_size, ownership, f))
    }

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    fn filter_devs<I>(dev_infos: I,
                      pool_uuid: &PoolUuid,
                      force: bool)
                      -> EngineResult<Vec<(Device, (PathBuf, Bytes, File))>>
        where I: Iterator<Item = (Device, EngineResult<(PathBuf, Bytes, DevOwnership, File)>)>
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            let (devnode, dev_size, ownership, f) = try!(dev_result);
            if dev_size < MIN_DEV_SIZE {
                let error_message = format!("{} too small, minimum {} bytes",
                                            devnode.display(),
                                            MIN_DEV_SIZE);
                return Err(EngineError::Engine(ErrorEnum::Invalid, error_message));
            };
            match ownership {
                DevOwnership::Unowned => add_devs.push((dev, (devnode, dev_size, f))),
                DevOwnership::Theirs => {
                    if !force {
                        let error_str = format!("First 8K of {} not zeroed", devnode.display());
                        return Err(EngineError::Engine(ErrorEnum::Invalid, error_str));
                    } else {
                        add_devs.push((dev, (devnode, dev_size, f)))
                    }
                }
                DevOwnership::Ours(uuid) => {
                    if *pool_uuid != uuid {
                        let error_str = format!("Device {} already belongs to Stratis pool {}",
                                                devnode.display(),
                                                uuid);
                        return Err(EngineError::Engine(ErrorEnum::Invalid, error_str));
                    }
                }
            }
        }
        Ok(add_devs)
    }

    try!(validate_mda_size(mda_size));

    let dev_infos = devices.into_iter().map(|d: Device| (d, dev_info(&d)));

    let add_devs = try!(filter_devs(dev_infos, pool_uuid, force));

    // TODO: Fix this code.  We should deal with any number of blockdevs
    //
    if add_devs.len() < 2 {
        return Err(EngineError::Engine(ErrorEnum::Error,
                                       "Need at least 2 blockdevs to create a pool".into()));
    }

    let mut bds = Vec::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {

        let bda = try!(BDA::initialize(&mut f,
                                       pool_uuid,
                                       &Uuid::new_v4(),
                                       mda_size,
                                       dev_size.sectors()));

        let bd = BlockDev {
            dev: dev,
            devnode: devnode.clone(),
            bda: bda,
        };
        bds.push(bd);
    }
    Ok(bds)
}


#[derive(Debug)]
pub struct BlockDev {
    dev: Device,
    pub devnode: PathBuf,
    bda: BDA,
}

impl BlockDev {
    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            total_size: self.size(),
        }
    }

    pub fn wipe_metadata(&mut self) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        BDA::wipe(&mut f)
    }

    /// Get the "x:y" device string for this blockdev
    pub fn dstr(&self) -> String {
        self.dev.dstr()
    }

    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        self.bda.save_state(time, metadata, &mut f)
    }

    pub fn load_state(&self) -> EngineResult<Option<Vec<u8>>> {
        let mut f = try!(OpenOptions::new().read(true).open(&self.devnode));
        self.bda.load_state(&mut f)
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (Sectors, Sectors) {
        let start = self.bda.size();
        let size = self.size();
        // Blockdev size is at least MIN_DEV_SIZE, so this can fail only if
        // size of metadata area exceeds 1 GiB. Initial metadata area size
        // is 4 MiB.
        assert!(start <= size);
        (start, size - start)
    }

    /// The /dev/mapper/<name> device is not immediately available for use.
    /// TODO: Implement wait for event or poll.
    pub fn wait_for_dm() {
        thread::sleep(Duration::from_millis(500))
    }

    /// The device's UUID.
    pub fn uuid(&self) -> &DevUuid {
        self.bda.dev_uuid()
    }

    /// The device's pool's UUID.
    pub fn pool_uuid(&self) -> &PoolUuid {
        self.bda.pool_uuid()
    }

    /// The device's size.
    pub fn size(&self) -> Sectors {
        self.bda.dev_size()
    }

    /// Last time metadata was written to this device.
    pub fn last_update_time(&self) -> &Option<Timespec> {
        self.bda.last_update_time()
    }
}
