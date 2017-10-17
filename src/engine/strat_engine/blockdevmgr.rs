// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use rand::{thread_rng, sample};
use uuid::Uuid;

use devicemapper::{Bytes, Device, IEC, Sectors, Segment};

use super::super::engine::BlockDev;
use super::super::errors::{EngineError, EngineResult, ErrorEnum};
use super::super::types::{DevUuid, PoolUuid};

use super::cleanup::wipe_blockdevs;
use super::blockdev::StratBlockDev;
use super::device::{blkdev_size, resolve_devices};
use super::engine::DevOwnership;
use super::metadata::{BDA, MIN_MDA_SECTORS, StaticHeader, validate_mda_size};
use super::range_alloc::RangeAllocator;
use super::serde_structs::{BlockDevSave, Recordable};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi);
const MAX_NUM_TO_WRITE: usize = 10;

#[derive(Debug)]
pub struct BlkDevSegment {
    pub uuid: DevUuid,
    pub segment: Segment,
}

impl BlkDevSegment {
    pub fn new(uuid: DevUuid, segment: Segment) -> BlkDevSegment {
        BlkDevSegment { uuid, segment }
    }

    pub fn to_segment(&self) -> Segment {
        self.segment.clone()
    }
}

impl Recordable<Vec<(Uuid, Sectors, Sectors)>> for Vec<BlkDevSegment> {
    fn record(&self) -> Vec<(Uuid, Sectors, Sectors)> {
        self.iter()
            .map(|bseg| (bseg.uuid, bseg.segment.start, bseg.segment.length))
            .collect::<Vec<_>>()
    }
}

/// Build a Vec<Segment> from BlkDevSegments. This is useful for calls
/// to the devicemapper library.
pub fn map_to_dm(bsegs: &[BlkDevSegment]) -> Vec<Segment> {
    bsegs
        .into_iter()
        .map(|bseg| bseg.to_segment())
        .collect::<Vec<_>>()
}


#[derive(Debug)]
pub struct BlockDevMgr {
    block_devs: Vec<StratBlockDev>,
    last_update_time: Option<DateTime<Utc>>,
}

impl BlockDevMgr {
    pub fn new(block_devs: Vec<StratBlockDev>) -> BlockDevMgr {
        BlockDevMgr {
            block_devs: block_devs,
            last_update_time: None,
        }
    }

    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(pool_uuid: PoolUuid,
                      paths: &[&Path],
                      mda_size: Sectors,
                      force: bool)
                      -> EngineResult<BlockDevMgr> {
        let devices = resolve_devices(paths)?;
        Ok(BlockDevMgr::new(initialize(pool_uuid, devices, mda_size, force)?))
    }

    /// Get a function that maps UUIDs to Devices.
    pub fn uuid_to_devno(&self) -> Box<Fn(DevUuid) -> Option<Device>> {
        let uuid_map: HashMap<DevUuid, Device> = self.block_devs
            .iter()
            .map(|bd| (bd.uuid(), *bd.device()))
            .collect();

        Box::new(move |uuid: DevUuid| -> Option<Device> { uuid_map.get(&uuid).cloned() })
    }

    pub fn add(&mut self,
               pool_uuid: PoolUuid,
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
        wipe_blockdevs(&self.block_devs)
    }

    /// Allocate space according to sizes vector request.
    /// Return the segments allocated for each request, or None if it was
    /// not possible to satisfy the request.
    /// This method is atomic, it either allocates all requested or allocates
    /// nothing.
    pub fn alloc_space(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<BlkDevSegment>>> {
        let total_needed: Sectors = sizes.iter().cloned().sum();
        if self.avail_space() < total_needed {
            return None;
        }

        let mut lists = Vec::new();
        for &needed in sizes {
            let mut alloc = Sectors(0);
            let mut segs = Vec::new();
            // TODO: Consider greater efficiency for allocation generally.
            // Over time, the blockdevs at the start will be exhausted. It
            // might be a good idea to keep an auxiliary structure, so that
            // only blockdevs with some space left to allocate are accessed.
            // In the context of this major inefficiency that ensues over time
            // the obvious but more minor inefficiency of this inner loop is
            // not worth worrying about.
            for bd in &mut self.block_devs {
                if alloc == needed {
                    break;
                }

                let (gotten, r_segs) = bd.request_space(needed - alloc);
                let blkdev_segs = r_segs
                    .into_iter()
                    .map(|(start, length)| {
                             BlkDevSegment::new(bd.uuid(),
                                                Segment::new(*bd.device(), start, length))
                         });
                segs.extend(blkdev_segs);
                alloc += gotten;
            }
            assert_eq!(alloc, needed);
            lists.push(segs);
        }

        Some(lists)
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

    /// Get references to managed blockdevs.
    pub fn blockdevs(&self) -> Vec<&BlockDev> {
        self.block_devs
            .iter()
            .map(|bd| bd as &BlockDev)
            .collect()
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

impl Recordable<HashMap<DevUuid, BlockDevSave>> for BlockDevMgr {
    fn record(&self) -> HashMap<Uuid, BlockDevSave> {
        self.block_devs
            .iter()
            .map(|bd| (bd.uuid(), bd.record()))
            .collect()
    }
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
pub fn initialize(pool_uuid: PoolUuid,
                  devices: HashMap<Device, &Path>,
                  mda_size: Sectors,
                  force: bool)
                  -> EngineResult<Vec<StratBlockDev>> {

    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    pub fn dev_info(devnode: &Path) -> EngineResult<(&Path, Bytes, DevOwnership, File)> {
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
    fn filter_devs<'a, I>(dev_infos: I,
                          pool_uuid: PoolUuid,
                          force: bool)
                          -> EngineResult<Vec<(Device, (&'a Path, Bytes, File))>>
        where I: Iterator<Item = (Device, EngineResult<(&'a Path, Bytes, DevOwnership, File)>)>
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
                    if pool_uuid != uuid {
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

    let dev_infos = devices.into_iter().map(|(d, p)| (d, dev_info(p)));

    let add_devs = filter_devs(dev_infos, pool_uuid, force)?;

    let mut bds: Vec<StratBlockDev> = Vec::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {

        let bda = BDA::initialize(&mut f,
                                  pool_uuid,
                                  Uuid::new_v4(),
                                  mda_size,
                                  dev_size.sectors(),
                                  Utc::now().timestamp() as u64);
        if let Ok(bda) = bda {
            let allocator = RangeAllocator::new(bda.dev_size(), &[(Sectors(0), bda.size())])
                .expect("bda.size() < bda.dev_size() and single range");

            bds.push(StratBlockDev::new(dev, devnode.to_owned(), bda, allocator));
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = BDA::wipe(&mut f);
            let _ = wipe_blockdevs(&bds);

            return Err(bda.unwrap_err());
        }
    }
    Ok(bds)
}
