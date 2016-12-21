// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;

use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;
use std::str::FromStr;

use devicemapper::Device;

use engine::{EngineResult, EngineError};

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


/// Locate Stratis devices using the find_all procedure.
mod setup {

    use std::io;

    use std::collections::BTreeMap;

    use std::fs::OpenOptions;
    use std::fs::read_dir;

    use std::io::ErrorKind;
    use std::io::Read;
    use std::io::Seek;
    use std::io::SeekFrom;

    use std::path::Path;

    use std::str::FromStr;

    use devicemapper::Device;
    use uuid::Uuid;

    use consts::SECTOR_SIZE;

    use engine::engine::EngineError;
    use engine::engine::EngineResult;
    use engine::engine::ErrorEnum;

    use types::Sectors;

    use super::super::blockdev::BlockDev;
    use super::super::engine::DevOwnership;
    use super::super::metadata::SigBlock;

    use super::blkdev_size;

    type DevUuid = Uuid;
    type PoolUuid = Uuid;

    /// If a Path refers to a valid Stratis blockdev, return a BlockDev
    /// struct. Otherwise, return None. Return an error if there was
    /// a problem inspecting the device.
    fn setup(devnode: &Path) -> EngineResult<Option<BlockDev>> {
        let dev = try!(Device::from_str(&devnode.to_string_lossy()));

        let mut f = try!(OpenOptions::new()
            .read(true)
            .open(devnode)
            .map_err(|_| {
                io::Error::new(ErrorKind::PermissionDenied,
                               format!("Could not open {}", devnode.display()))
            }));

        let mut buf = [0u8; 4096];
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.read(&mut buf));

        match SigBlock::determine_ownership(&buf) {
            Ok(DevOwnership::Ours(_)) => {}
            Ok(_) => {
                return Ok(None);
            }
            Err(err) => {
                let error_message = format!("{} for devnode {}", err, devnode.display());
                return Err(EngineError::Stratis(ErrorEnum::Invalid(error_message)));
            }
        };

        Ok(Some(BlockDev {
            dev: dev,
            devnode: devnode.to_owned(),
            sigblock: match SigBlock::read(&buf, 0, Sectors(try!(blkdev_size(&f)) / SECTOR_SIZE)) {
                Ok(sigblock) => sigblock,
                Err(err) => {
                    return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
                }
            },
        }))
    }


    /// Find all Stratis Blockdevs.
    ///
    /// Returns a map of pool uuids to maps of blockdev uuids to blockdevs.
    pub fn find_all() -> EngineResult<BTreeMap<PoolUuid, BTreeMap<DevUuid, BlockDev>>> {
        let mut pool_map = BTreeMap::new();
        for dir_e in try!(read_dir("/dev")) {
            let devnode = match dir_e {
                Ok(d) => d.path(),
                Err(_) => continue,
            };

            match setup(&devnode) {
                Ok(Some(blockdev)) => {
                    pool_map.entry(blockdev.sigblock.pool_uuid)
                        .or_insert_with(BTreeMap::new)
                        .insert(blockdev.sigblock.dev_uuid, blockdev);
                }
                _ => continue,
            };
        }

        Ok(pool_map)
    }
}


/// Initialize blockdevs as Stratis devs using the initialize procedure.
pub mod initialize {

    use std::io;

    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    use std::fs::OpenOptions;

    use std::io::ErrorKind;
    use std::io::Read;
    use std::io::Seek;
    use std::io::SeekFrom;

    use std::path::PathBuf;

    use devicemapper::Device;
    use uuid::Uuid;

    use consts::SECTOR_SIZE;

    use types::Sectors;

    use engine::engine::EngineError;
    use engine::engine::EngineResult;
    use engine::engine::ErrorEnum;

    use super::super::blockdev::BlockDev;
    use super::super::consts::MIN_DEV_SIZE;
    use super::super::engine::DevOwnership;
    use super::super::metadata::SigBlock;
    use super::super::metadata::validate_mda_size;

    use super::blkdev_size;

    type DevUuid = Uuid;
    type PoolUuid = Uuid;

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    fn filter_devs<I>(dev_infos: I,
                      pool_uuid: &PoolUuid,
                      force: bool)
                      -> EngineResult<Vec<(Device, (PathBuf, u64))>>
        where I: Iterator<Item = (Device, EngineResult<(PathBuf, u64, DevOwnership)>)>
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            if dev_result.is_err() {
                return Err(dev_result.unwrap_err());
            }
            let (devnode, dev_size, ownership) = dev_result.unwrap();
            if dev_size < MIN_DEV_SIZE {
                let error_message = format!("{} too small, minimum {} bytes",
                                            devnode.display(),
                                            MIN_DEV_SIZE);
                return Err(EngineError::Stratis(ErrorEnum::Invalid(error_message)));
            };
            match ownership {
                DevOwnership::Unowned => add_devs.push((dev, (devnode, dev_size))),
                DevOwnership::Theirs => {
                    if !force {
                        let error_str = format!("First 4K of {} not zeroed", devnode.display());
                        return Err(EngineError::Stratis(ErrorEnum::Invalid(error_str)));
                    } else {
                        add_devs.push((dev, (devnode, dev_size)))
                    }
                }
                DevOwnership::Ours(uuid) => {
                    if *pool_uuid != uuid {
                        let error_str = format!("Device {} already belongs to Stratis pool {}",
                                                devnode.display(),
                                                uuid);
                        return Err(EngineError::Stratis(ErrorEnum::Invalid(error_str)));
                    }
                }
            }
        }
        Ok(add_devs)
    }

    /// Initialize multiple blockdevs at once. This allows all of them
    /// to be checked for usability before writing to any of them.
    pub fn initialize(pool_uuid: &PoolUuid,
                      devices: BTreeSet<Device>,
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<BTreeMap<PathBuf, BlockDev>> {

        match validate_mda_size(mda_size) {
            None => {}
            Some(err) => {
                return Err(EngineError::Stratis(ErrorEnum::Invalid(err)));
            }
        };

        let dev_infos = devices.into_iter().map(|d: Device| (d, dev_info(&d)));

        let add_devs = try!(filter_devs(dev_infos, pool_uuid, force));

        let mut bds = BTreeMap::new();
        for (dev, (devnode, dev_size)) in add_devs {

            let bd = BlockDev {
                dev: dev,
                devnode: devnode.clone(),
                sigblock: SigBlock::new(pool_uuid,
                                        &Uuid::new_v4(),
                                        mda_size,
                                        Sectors(dev_size / SECTOR_SIZE)),
            };

            try!(bd.write_sigblock());
            bds.insert(devnode, bd);
        }
        Ok(bds)
    }

    /// Gets device information, returns an error if problem with obtaining
    /// that information.
    fn dev_info(dev: &Device) -> EngineResult<(PathBuf, u64, DevOwnership)> {
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

        let mut buf = [0u8; 4096];
        try!(f.seek(SeekFrom::Start(0)));
        try!(f.read(&mut buf));

        let ownership = match SigBlock::determine_ownership(&buf) {
            Ok(ownership) => ownership,
            Err(err) => {
                let error_message = format!("{} for device {}", err, devnode.display());
                return Err(EngineError::Stratis(ErrorEnum::Invalid(error_message)));
            }
        };

        Ok((devnode, dev_size, ownership))
    }
}
