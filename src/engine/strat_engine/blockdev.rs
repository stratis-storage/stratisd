// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::collections::BTreeSet;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::ErrorKind;
use std::fs::{OpenOptions, read_dir};
use std::os::unix::prelude::AsRawFd;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use time::Timespec;
use devicemapper::Device;
use uuid::Uuid;

use types::Sectors;
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::*;
use super::metadata::{StaticHeader, BDA, validate_mda_size, BDA_STATIC_HDR_SECTORS};
use super::engine::DevOwnership;
pub use super::BlockDevSave;

type DevUuid = Uuid;
type PoolUuid = Uuid;

const MIN_DEV_SIZE: u64 = GIGA;

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<u64> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(val),
    }
}

/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
pub fn resolve_devices(paths: &[&Path]) -> io::Result<BTreeSet<Device>> {
    let mut devices = BTreeSet::new();
    for path in paths {
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        devices.insert(dev);
    }
    Ok(devices)
}

/// Find all Stratis Blockdevs.
///
/// Returns a map of pool uuids to maps of blockdev uuids to blockdevs.
pub fn find_all() -> EngineResult<BTreeMap<PoolUuid, BTreeMap<DevUuid, BlockDev>>> {

    /// If a Path refers to a valid Stratis blockdev, return a BlockDev
    /// struct. Otherwise, return None. Return an error if there was
    /// a problem inspecting the device.
    fn setup(devnode: &Path) -> EngineResult<Option<BlockDev>> {
        let dev = try!(Device::from_str(&devnode.to_string_lossy()));

        let mut f = try!(OpenOptions::new()
            .read(true)
            .open(devnode));

        let static_header = try!(StaticHeader::setup(&mut f));
        let bda = try!(BDA::load(&mut f, static_header));

        Ok(Some(BlockDev {
            dev: dev,
            devnode: devnode.to_owned(),
            bda: bda,
        }))
    }

    let mut pool_map = BTreeMap::new();
    for dir_e in try!(read_dir("/dev")) {
        let devnode = match dir_e {
            Ok(d) => d.path(),
            Err(_) => continue,
        };

        match setup(&devnode) {
            Ok(Some(blockdev)) => {
                pool_map.entry(blockdev.bda.header.pool_uuid)
                    .or_insert_with(BTreeMap::new)
                    .insert(blockdev.bda.header.dev_uuid, blockdev);
            }
            _ => continue,
        };
    }

    Ok(pool_map)
}



/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
pub fn initialize(pool_uuid: &PoolUuid,
                  devices: BTreeSet<Device>,
                  mda_size: Sectors,
                  force: bool)
                  -> EngineResult<BTreeMap<PathBuf, BlockDev>> {

    /// Gets device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the blockdev's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    fn dev_info(dev: &Device) -> EngineResult<(PathBuf, u64, DevOwnership, File)> {
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
                      -> EngineResult<Vec<(Device, (PathBuf, u64, File))>>
        where I: Iterator<Item = (Device, EngineResult<(PathBuf, u64, DevOwnership, File)>)>
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            if dev_result.is_err() {
                return Err(dev_result.unwrap_err());
            }
            let (devnode, dev_size, ownership, f) = dev_result.unwrap();
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

    let mut bds = BTreeMap::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {

        let static_header = StaticHeader::new(pool_uuid,
                                              &Uuid::new_v4(),
                                              mda_size,
                                              Sectors(dev_size / SECTOR_SIZE));
        let bda = try!(BDA::initialize(&mut f, static_header));

        let bd = BlockDev {
            dev: dev,
            devnode: devnode.clone(),
            bda: bda,
        };
        bds.insert(devnode, bd);
    }
    Ok(bds)
}


#[derive(Debug)]
pub struct BlockDev {
    pub dev: Device,
    pub devnode: PathBuf,
    pub bda: BDA,
}

impl BlockDev {
    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            total_size: self.bda.header.blkdev_size,
        }
    }

    /// Get the "x:y" device string for this blockdev
    pub fn dstr(&self) -> String {
        self.dev.dstr()
    }

    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        try!(self.bda.save_state(time, metadata, &mut f));

        Ok(())
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (Sectors, Sectors) {
        let start = Sectors(BDA_STATIC_HDR_SECTORS) + self.bda.header.mda_size +
                    self.bda.header.reserved_size;
        let length = self.bda.header.blkdev_size - start;
        (start, length)
    }
}
