// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::io;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::ErrorKind;
use std::io::{Seek, Write, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::os::unix::prelude::AsRawFd;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::Duration;

use devicemapper::consts::SECTOR_SIZE;
use devicemapper::Device;
use devicemapper::Segment;
use devicemapper::{Bytes, Sectors};
use time::Timespec;
use uuid::Uuid;

use consts::IEC;
use engine::{DevUuid, EngineResult, EngineError, ErrorEnum, PoolUuid};
use super::metadata::{StaticHeader, BDA, validate_mda_size};
use super::engine::DevOwnership;
pub use super::BlockDevSave;
use engine::strat_engine::range_alloc::RangeAllocator;

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi as u64);

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(Bytes(val)),
    }
}

/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
pub fn resolve_devices(paths: &[&Path]) -> io::Result<HashSet<Device>> {
    let mut devices = HashSet::new();
    for path in paths {
        let dev = try!(Device::from_str(&path.to_string_lossy()));
        devices.insert(dev);
    }
    Ok(devices)
}

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
            // TODO: Parse MDA and also initialize RangeAllocator with
            // in-use regions
            let allocator = RangeAllocator::new_with_used(bda.dev_size(),
                                                          &[(Sectors(0), bda.size())]);
            Ok(Some(BlockDev {
                dev: dev,
                devnode: devnode.to_owned(),
                bda: bda,
                used: allocator,
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

/// Write buf at offset length times.
pub fn write_sectors(path: &Path,
                     offset: Sectors,
                     length: Sectors,
                     buf: &[u8; SECTOR_SIZE])
                     -> EngineResult<()> {
    let mut f = try!(OpenOptions::new().write(true).open(path));

    try!(f.seek(SeekFrom::Start(*offset)));
    for _ in 0..*length {
        try!(f.write_all(buf));
    }

    try!(f.flush());
    Ok(())
}

/// Zero sectors at the given offset for length sectors.
pub fn wipe_sectors(path: &Path, offset: Sectors, length: Sectors) -> EngineResult<()> {
    write_sectors(path, offset, length, &[0u8; SECTOR_SIZE])
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
pub fn initialize(pool_uuid: &PoolUuid,
                  devices: HashSet<Device>,
                  mda_size: Sectors,
                  force: bool)
                  -> EngineResult<Vec<BlockDev>> {
    /// Gets device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the blockdev's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    fn dev_info(dev: &Device) -> EngineResult<(PathBuf, Bytes, DevOwnership, File)> {
        let devnode = try!(dev.devnode().ok_or_else(|| {
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

    let mut bds = Vec::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {

        let bda = try!(BDA::initialize(&mut f,
                                       pool_uuid,
                                       &Uuid::new_v4(),
                                       mda_size,
                                       dev_size.sectors()));
        let allocator = RangeAllocator::new_with_used(bda.dev_size(), &[(Sectors(0), bda.size())]);

        let bd = BlockDev {
            dev: dev,
            devnode: devnode.clone(),
            bda: bda,
            used: allocator,
        };
        bds.push(bd);
    }
    Ok(bds)
}


#[derive(Debug)]
pub struct BlockDev {
    pub dev: Device,
    pub devnode: PathBuf,
    bda: BDA,
    used: RangeAllocator,
}

impl BlockDev {
    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            total_size: self.size(),
        }
    }

    pub fn wipe_metadata(self) -> EngineResult<()> {
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
    fn avail_range(&self) -> (Sectors, Sectors) {
        let start = self.bda.size();
        let size = self.size();
        // Blockdev size is at least MIN_DEV_SIZE, so this can fail only if
        // size of metadata area exceeds 1 GiB. Initial metadata area size
        // is 4 MiB.
        assert!(start <= size);
        (start, size - start)
    }

    /// Return the available range as a segment
    pub fn avail_range_segment(&self) -> Segment {
        let (start, length) = self.avail_range();
        Segment::new(self.dev, start, length)
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
    pub fn last_update_time(&self) -> Option<&Timespec> {
        self.bda.last_update_time()
    }

    pub fn available(&self) -> Sectors {
        self.used.available()
    }

    // Find some sector ranges that could be allocated. If more
    // sectors are needed than our capacity, return partial results.
    pub fn request_space(&mut self, size: Sectors) -> (Sectors, Vec<(Sectors, Sectors)>) {
        self.used.request(size)
    }
}
