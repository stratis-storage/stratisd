// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::Path,
};

use chrono::{DateTime, Duration, Utc};
use rand::{seq, thread_rng};

use devicemapper::{Bytes, Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        engine::BlockDev,
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                devices::{initialize_devices, process_devices, wipe_blockdevs, DeviceInfo},
                metadata::MDADataSize,
            },
            serde_structs::{BaseBlockDevSave, BaseDevSave, Recordable},
        },
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const MAX_NUM_TO_WRITE: usize = 10;

/// struct to represent a continuous set of sectors on a disk
#[derive(Debug, Clone)]
pub struct Segment {
    /// The offset into the device where this segment starts.
    pub(super) start: Sectors,
    /// The length of the segment.
    pub(super) length: Sectors,
    /// The device the segment is within.
    pub(super) device: Device,
}

impl Segment {
    /// Create a new Segment with given attributes
    pub fn new(device: Device, start: Sectors, length: Sectors) -> Segment {
        Segment {
            device,
            start,
            length,
        }
    }
}

#[derive(Clone)]
pub struct BlkDevSegment {
    pub(super) uuid: DevUuid,
    pub(super) segment: Segment,
}

impl BlkDevSegment {
    pub fn new(uuid: DevUuid, segment: Segment) -> BlkDevSegment {
        BlkDevSegment { uuid, segment }
    }

    pub fn to_segment(&self) -> Segment {
        self.segment.clone()
    }
}

impl fmt::Debug for BlkDevSegment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("BlkDevSegment")
            .field("uuid", &self.uuid.to_simple_ref())
            .field("segment", &self.segment)
            .finish()
    }
}

impl Recordable<Vec<BaseDevSave>> for Vec<BlkDevSegment> {
    fn record(&self) -> Vec<BaseDevSave> {
        self.iter()
            .map(|bseg| BaseDevSave {
                parent: bseg.uuid,
                start: bseg.segment.start,
                length: bseg.segment.length,
            })
            .collect::<Vec<_>>()
    }
}

/// Build a linear dev target table from BlkDevSegments. This is useful for
/// calls to the devicemapper library.
pub fn map_to_dm(bsegs: &[BlkDevSegment]) -> Vec<TargetLine<LinearDevTargetParams>> {
    let mut table = Vec::new();
    let mut logical_start_offset = Sectors(0);

    let segments = bsegs
        .iter()
        .map(|bseg| bseg.to_segment())
        .collect::<Vec<_>>();
    for segment in segments {
        let (physical_start_offset, length) = (segment.start, segment.length);
        let params = LinearTargetParams::new(segment.device, physical_start_offset);
        let line = TargetLine::new(
            logical_start_offset,
            length,
            LinearDevTargetParams::Linear(params),
        );
        table.push(line);
        logical_start_offset += length;
    }

    table
}

#[derive(Debug)]
pub struct BlockDevMgr {
    block_devs: Vec<StratBlockDev>,
    last_update_time: Option<DateTime<Utc>>,
}

// Check coherence of pool and device UUIDs with existing state of current
// blockdevmgr. If the selection of devices is incompatible with the current
// state of the blockdevmgr, or simply invalid, return an error.
//
// Postcondition: All infos in the returned vector have their
// stratis_identifiers value equal to None. Either their Stratis identifiers
// indicated an error, or else the devices specified are already owned by
// this blockdevmgr and should not be added again.
//
// FIXME:
// Note that this method _should_ be somewhat temporary. We hope that in
// another step the functionality contained will be hoisted up closer to
// the D-Bus/engine interface, as it computes some idempotency information.
fn check_device_ids(
    pool_uuid: PoolUuid,
    current_uuids: &HashSet<DevUuid>,
    devices: Vec<DeviceInfo>,
) -> StratisResult<Vec<DeviceInfo>> {
    let stratis_identifiers: HashMap<PoolUuid, HashSet<DevUuid>> = devices
        .iter()
        .filter_map(|info| info.stratis_identifiers)
        .fold(HashMap::new(), |mut acc, (pool_uuid, dev_uuid)| {
            acc.entry(pool_uuid)
                .or_insert_with(HashSet::new)
                .insert(dev_uuid);
            acc
        });

    let (this_pool, other_pools): (Vec<_>, Vec<_>) = stratis_identifiers
        .iter()
        .partition(|(k, _)| **k == pool_uuid);

    if !other_pools.is_empty() {
        let error_string = other_pools
            .iter()
            .map(|(p, devs)| {
                let dev_string = devices
                    .iter()
                    .filter(|info| match info.stratis_identifiers {
                        None => false,
                        Some((pool_uuid, _)) => devs.contains(&pool_uuid),
                    })
                    .map(|info| info.devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Devices ({}) appear to belong to Stratis pool with UUID {}",
                    dev_string,
                    p.to_simple_ref()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let error_message = format!(
            "Some devices specified appear to be already in use by other Stratis pools: {}",
            error_string
        );
        return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
    }

    if !this_pool.is_empty() {
        let (_, dev_uuids) = this_pool[0];

        let invalid_uuids = dev_uuids.difference(current_uuids).collect::<Vec<_>>();

        if !invalid_uuids.is_empty() {
            let error_string = devices
                .iter()
                .filter(|info| match info.stratis_identifiers {
                    None => false,
                    Some((pool_uuid, _)) => invalid_uuids.contains(&&pool_uuid),
                })
                .map(|info| info.devnode.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let error_message = format!(
                "Devices ({}) appear to be already in use by this pool; they may be in use by the other tier",
                error_string
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
        }
    }

    Ok(devices
        .into_iter()
        .filter(|info| info.stratis_identifiers.is_none())
        .collect())
}

impl BlockDevMgr {
    /// Make a struct that represents an existing BlockDevMgr.
    pub fn new(
        block_devs: Vec<StratBlockDev>,
        last_update_time: Option<DateTime<Utc>>,
    ) -> BlockDevMgr {
        BlockDevMgr {
            block_devs,
            last_update_time,
        }
    }

    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(
        pool_uuid: PoolUuid,
        paths: &[&Path],
        mda_data_size: MDADataSize,
    ) -> StratisResult<BlockDevMgr> {
        let devices = check_device_ids(pool_uuid, &HashSet::new(), process_devices(paths)?)?;

        Ok(BlockDevMgr::new(
            initialize_devices(devices, pool_uuid, mda_data_size)?,
            None,
        ))
    }

    /// Get a hashmap that maps UUIDs to Devices.
    pub fn uuid_to_devno(&self) -> HashMap<DevUuid, Device> {
        self.block_devs
            .iter()
            .map(|bd| (bd.uuid(), *bd.device()))
            .collect()
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(&mut self, pool_uuid: PoolUuid, paths: &[&Path]) -> StratisResult<Vec<DevUuid>> {
        let current_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.uuid())
            .collect::<HashSet<_>>();
        let devices = check_device_ids(pool_uuid, &current_uuids, process_devices(paths)?)?;

        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize_devices(devices, pool_uuid, MDADataSize::default())?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_uuids)
    }

    pub fn destroy_all(&mut self) -> StratisResult<()> {
        wipe_blockdevs(&self.block_devs)
    }

    /// Remove the specified block devs and erase their metadata.
    ///
    /// Precondition: It is the responsibility of the caller to ensure that
    /// none of the blockdevs are in use, that is, have had any space allocated
    /// from them for upper layers.
    ///
    /// If a specified blockdev is not found, returns an error and does nothing.
    ///
    /// NOTE: This method traverses the block_devs Vec from the rear to the
    /// front, looking for blockdevs to remove. This is algorithmically
    /// inefficient, unless it is assumed that the blockdevs specified are very
    /// near the end of the Vec, which is expected to be the case. In that case,
    /// the algorithm is O(n).
    pub(super) fn remove_blockdevs(&mut self, uuids: &[DevUuid]) -> StratisResult<()> {
        let mut removed = Vec::new();
        for uuid in uuids {
            let mut found = false;
            let blockdevs_last_index = self.block_devs.len() - 1;
            for i in 0..blockdevs_last_index {
                let index = blockdevs_last_index - i;
                if self.block_devs[index].uuid() == *uuid {
                    removed.push(self.block_devs.swap_remove(index));
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(StratisError::Engine(
                    ErrorEnum::Error,
                    format!(
                        "Blockdev corresponding to UUID: {} not found.",
                        uuid.to_simple_ref()
                    ),
                ));
            }
        }
        wipe_blockdevs(&removed)?;
        Ok(())
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
                let blkdev_segs = r_segs.into_iter().map(|(start, length)| {
                    BlkDevSegment::new(bd.uuid(), Segment::new(*bd.device(), start, length))
                });
                segs.extend(blkdev_segs);
                alloc += gotten;
            }
            assert_eq!(alloc, needed);
            lists.push(segs);
        }

        Some(lists)
    }

    /// Write the given data to all blockdevs marking with current time.
    /// Return an error if data was not written to any blockdev.
    /// Omit blockdevs which do not have sufficient space in BDA to accommodate
    /// metadata. If current time is not more recent than previously written
    /// time, use a time that is one nanosecond greater than that previously
    /// written. Randomly select no more than MAX_NUM_TO_WRITE blockdevs to
    /// write to.
    pub fn save_state(&mut self, metadata: &[u8]) -> StratisResult<()> {
        let current_time = Utc::now();
        let stamp_time = if Some(current_time) <= self.last_update_time {
            self.last_update_time
                .expect("self.last_update_time >= Some(current_time")
                .checked_add_signed(Duration::nanoseconds(1))
                .expect("self.last_update_time << maximum representable DateTime")
        } else {
            current_time
        };

        let data_size = Bytes(metadata.len() as u64);
        let candidates = self
            .block_devs
            .iter_mut()
            .filter(|b| b.max_metadata_size().bytes() >= data_size);

        // TODO: consider making selection not entirely random, i.e, ensuring
        // distribution of metadata over different paths.
        let saved = seq::sample_iter(&mut thread_rng(), candidates, MAX_NUM_TO_WRITE)
            .unwrap_or_else(|e| e)
            .iter_mut()
            .fold(false, |acc, b| {
                acc | b.save_state(&stamp_time, metadata).is_ok()
            });

        if saved {
            self.last_update_time = Some(stamp_time);
            Ok(())
        } else {
            let err_msg = "Failed to save metadata to even one device in pool";
            Err(StratisError::Engine(ErrorEnum::Error, err_msg.into()))
        }
    }

    /// Get references to managed blockdevs.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &StratBlockDev)> {
        self.block_devs.iter().map(|bd| (bd.uuid(), bd)).collect()
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut StratBlockDev)> {
        self.block_devs
            .iter_mut()
            .map(|bd| (bd.uuid(), bd as &mut StratBlockDev))
            .collect()
    }

    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&StratBlockDev> {
        self.block_devs.iter().find(|bd| bd.uuid() == uuid)
    }

    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut StratBlockDev> {
        self.block_devs.iter_mut().find(|bd| bd.uuid() == uuid)
    }

    // SIZE methods

    /// The number of sectors not allocated for any purpose.
    pub fn avail_space(&self) -> Sectors {
        self.block_devs.iter().map(|bd| bd.available()).sum()
    }

    /// The current size of all the blockdevs.
    /// self.size() > self.avail_space() because some sectors are certainly
    /// allocated for Stratis metadata
    pub fn size(&self) -> Sectors {
        self.block_devs.iter().map(|b| b.size()).sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.size() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|bd| bd.metadata_size().sectors())
            .sum()
    }
}

impl Recordable<Vec<BaseBlockDevSave>> for BlockDevMgr {
    fn record(&self) -> Vec<BaseBlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::engine::strat_engine::{
        cmd,
        tests::{loopbacked, real},
    };

    use super::*;

    /// Verify that initially,
    /// size() - metadata_size() = avail_space().
    /// After 2 Sectors have been allocated, that amount must also be included
    /// in balance.
    fn test_blockdevmgr_used(paths: &[&Path]) {
        let mut mgr =
            BlockDevMgr::initialize(Uuid::new_v4(), paths, MDADataSize::default()).unwrap();
        assert_eq!(mgr.avail_space() + mgr.metadata_size(), mgr.size());

        let allocated = Sectors(2);
        mgr.alloc_space(&[allocated]).unwrap();
        assert_eq!(
            mgr.avail_space() + allocated + mgr.metadata_size(),
            mgr.size()
        );
    }

    #[test]
    pub fn loop_test_blockdevmgr_used() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_blockdevmgr_used,
        );
    }

    #[test]
    pub fn real_test_blockdevmgr_used() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_blockdevmgr_used,
        );
    }

    #[test]
    pub fn travis_test_blockdevmgr_used() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_blockdevmgr_used,
        );
    }

    /// Verify that it is impossible to steal blockdevs from another Stratis
    /// pool.
    /// 1. Initialize devices with pool uuid.
    /// 2. Initializing again with different uuid must fail.
    /// 3. Adding the devices must succeed, because they already belong.
    fn test_initialization_add_stratis(paths: &[&Path]) {
        assert!(paths.len() > 1);
        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let uuid = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();

        let mut bd_mgr = BlockDevMgr::initialize(uuid, paths1, MDADataSize::default()).unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(
            BlockDevMgr::initialize(uuid2, paths1, MDADataSize::default()),
            Err(_)
        );

        let original_length = bd_mgr.block_devs.len();
        assert_matches!(bd_mgr.add(uuid, paths1), Ok(_));
        assert_eq!(bd_mgr.block_devs.len(), original_length);

        BlockDevMgr::initialize(uuid, paths2, MDADataSize::default()).unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(bd_mgr.add(uuid, paths2), Err(_));
    }

    #[test]
    pub fn loop_test_initialization_stratis() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialization_add_stratis,
        );
    }

    #[test]
    pub fn real_test_initialization_stratis() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_initialization_add_stratis,
        );
    }

    #[test]
    pub fn travis_test_initialization_stratis() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialization_add_stratis,
        );
    }
}
