// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::io;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use rand::{thread_rng, sample};
use uuid::Uuid;

use devicemapper::{Bytes, Device, Sectors, Segment};

use super::super::consts::IEC;
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{DevUuid, PoolUuid};

use super::cleanup::wipe_blockdevs;
use super::blockdev::BlockDev;
use super::device::blkdev_size;
use super::engine::DevOwnership;
use super::metadata::{BDA, MIN_MDA_SECTORS, StaticHeader, validate_mda_size};
use super::range_alloc::RangeAllocator;
use super::serde_structs::{BlockDevSave, Recordable};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi);
const MAX_NUM_TO_WRITE: usize = 10;

/// Resolve a list of Paths of some sort to a set of unique Devices.
/// Return an IOError if there was a problem resolving any particular device.
pub fn resolve_devices(paths: &[&Path]) -> io::Result<HashSet<Device>> {
    paths
        .iter()
        .map(|p| Device::from_str(&p.to_string_lossy()))
        .collect()
}


#[derive(Debug)]
pub struct BlockDevMgr {
    block_devs: Vec<BlockDev>,
    last_update_time: Option<DateTime<Utc>>,
}

impl BlockDevMgr {
    pub fn new(block_devs: Vec<BlockDev>) -> BlockDevMgr {
        BlockDevMgr {
            block_devs: block_devs,
            last_update_time: None,
        }
    }

    /// Initialize a new BlockDevMgr with specified pool and devices.
    pub fn initialize(pool_uuid: &PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<BlockDevMgr> {
        let devices = resolve_devices(paths)?;
        Ok(BlockDevMgr::new(initialize(pool_uuid, devices, mda_size, force)?))
    }

    /// Obtain a BlockDev by its Device.
    pub fn get_by_device(&self, device: Device) -> Option<&BlockDev> {
        self.block_devs.iter().find(|d| d.device() == &device)
    }

    // Obtain a BlockDev by its UUID.
    pub fn get_by_uuid(&self, uuid: &DevUuid) -> Option<&BlockDev> {
        self.block_devs.iter().find(|d| d.uuid() == uuid)
    }

    pub fn add(&mut self,
               pool_uuid: &PoolUuid,
               paths: &[&Path],
               force: bool)
               -> EngineResult<Vec<PathBuf>> {
        let devices = resolve_devices(paths)?;
        let bds = initialize(pool_uuid, devices, MIN_MDA_SECTORS, force)?;
        let bdev_paths = bds.iter().map(|p| p.devnode.clone()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_paths)
    }

    pub fn destroy_all(self) -> EngineResult<()> {
        wipe_blockdevs(self.block_devs)
    }

    /// If available space is less than size, return None, else return
    /// the segments allocated.
    pub fn alloc_space(&mut self, size: Sectors) -> Option<Vec<Segment>> {
        let mut needed: Sectors = size;
        let mut segs = Vec::new();

        if self.avail_space() < size {
            return None;
        }

        for bd in &mut self.block_devs {
            if needed == Sectors(0) {
                break;
            }

            let (gotten, r_segs) = bd.request_space(needed);
            segs.extend(r_segs);
            needed -= gotten;
        }

        assert_eq!(needed, Sectors(0));

        Some(segs)
    }

    pub fn devnodes(&self) -> Vec<PathBuf> {
        self.block_devs
            .iter()
            .map(|d| d.devnode.clone())
            .collect()
    }

    /// Write the given data to all blockdevs marking with current time.
    /// Return an error if data was not written to any blockdev.
    /// Omit blockdevs which do not have sufficient space in BDA to accommodate
    /// metadata. If current time is not more recent than previously written
    /// time, use a time that is one nanosecond greater than that previously
    /// written. Randomly select no more than MAX_NUM_TO_WRITE blockdevs to
    /// write to.
    pub fn save_state(&mut self, metadata: &[u8]) -> EngineResult<()> {
        let current_time = Utc::now();
        let stamp_time = if Some(current_time) <= self.last_update_time {
            self.last_update_time
                .expect("self.last_update_time >= Some(current_time")
                .checked_add_signed(Duration::nanoseconds(1))
                .expect("self.last_update_time << maximum representable DateTime")
        } else {
            current_time
        };

        let data_size = Bytes(metadata.len() as u64).sectors();
        let candidates = self.block_devs
            .iter_mut()
            .filter(|b| b.max_metadata_size() >= data_size);

        // TODO: consider making selection not entirely random, i.e, ensuring
        // distribution of metadata over different paths.
        let saved = sample(&mut thread_rng(), candidates, MAX_NUM_TO_WRITE)
            .iter_mut()
            .fold(false,
                  |acc, b| acc | b.save_state(&stamp_time, metadata).is_ok());

        if saved {
            self.last_update_time = Some(stamp_time);
            Ok(())
        } else {
            let err_msg = "Failed to save metadata to even one device in pool";
            Err(EngineError::Engine(ErrorEnum::Error, err_msg.into()))
        }
    }

    // SIZE methods

    /// The number of sectors not allocated for any purpose.
    pub fn avail_space(&self) -> Sectors {
        self.block_devs.iter().map(|bd| bd.available()).sum()
    }

    /// The current capacity of all the blockdevs.
    /// self.current_capacity() > self.avail_space() because some sectors
    /// are certainly allocated for Stratis metadata
    pub fn current_capacity(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|b| b.current_capacity())
            .sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.current_capacity() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|bd| bd.metadata_size())
            .sum()
    }
}

impl Recordable<HashMap<Uuid, BlockDevSave>> for BlockDevMgr {
    fn record(&self) -> EngineResult<HashMap<Uuid, BlockDevSave>> {
        self.block_devs
            .iter()
            .map(|bd| bd.record().and_then(|bdsave| Ok((*bd.uuid(), bdsave))))
            .collect()
    }
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
pub fn initialize(pool_uuid: &PoolUuid,
                  devices: HashSet<Device>,
                  mda_size: Sectors,
                  force: bool)
                  -> EngineResult<Vec<BlockDev>> {

    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    pub fn dev_info(dev: &Device) -> EngineResult<(PathBuf, Bytes, DevOwnership, File)> {
        let devnode = dev.devnode()
            .ok_or_else(|| {
                            EngineError::Engine(ErrorEnum::NotFound,
                                                format!("could not get device node from dev {}",
                                                        dev.dstr()))
                        })?;

        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&devnode)?;
        let dev_size = blkdev_size(&f)?;
        let ownership = StaticHeader::determine_ownership(&mut f)?;

        Ok((devnode, dev_size, ownership, f))
    }

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    #[allow(type_complexity)]
    fn filter_devs<I>(dev_infos: I,
                      pool_uuid: &PoolUuid,
                      force: bool)
                      -> EngineResult<Vec<(Device, (PathBuf, Bytes, File))>>
        where I: Iterator<Item = (Device, EngineResult<(PathBuf, Bytes, DevOwnership, File)>)>
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            let (devnode, dev_size, ownership, f) = dev_result?;
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
                        let err_str = format!("Device {} appears to belong to another application",
                                              devnode.display());
                        return Err(EngineError::Engine(ErrorEnum::Invalid, err_str));
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
                    } else {
                        // Already in this pool (according to its header)
                        // TODO: Check we already know about it
                        // if yes, ignore. If no, add it w/o initializing?
                    }
                }
            }
        }
        Ok(add_devs)
    }

    validate_mda_size(mda_size)?;

    let dev_infos = devices.into_iter().map(|d: Device| (d, dev_info(&d)));

    let add_devs = filter_devs(dev_infos, pool_uuid, force)?;

    let mut bds: Vec<BlockDev> = Vec::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {

        let bda = BDA::initialize(&mut f,
                                  pool_uuid,
                                  &Uuid::new_v4(),
                                  mda_size,
                                  dev_size.sectors());
        if let Ok(bda) = bda {
            let allocator = RangeAllocator::new(bda.dev_size(), &[(Sectors(0), bda.size())])
                .expect("bda.size() < bda.dev_size() and single range");

            bds.push(BlockDev::new(dev, devnode, bda, allocator));
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = BDA::wipe(&mut f);
            let _ = wipe_blockdevs(bds);

            return Err(bda.unwrap_err());
        }
    }
    Ok(bds)
}
