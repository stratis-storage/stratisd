// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use rand::{seq, thread_rng};
use uuid::Uuid;

use devicemapper::{Bytes, Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine,
                   IEC};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::types::{DevUuid, PoolUuid};

use super::super::engine::DevOwnership;
use super::super::serde_structs::{BlockDevSave, Recordable};

use super::blockdev::StratBlockDev;
use super::cleanup::wipe_blockdevs;
use super::device::{blkdev_size, resolve_devices};
use super::metadata::{validate_mda_size, StaticHeader, BDA, MIN_MDA_SECTORS};
use super::util::hw_lookup;

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi);
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

#[derive(Clone, Debug)]
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

impl Recordable<Vec<(Uuid, Sectors, Sectors)>> for Vec<BlkDevSegment> {
    fn record(&self) -> Vec<(Uuid, Sectors, Sectors)> {
        self.iter()
            .map(|bseg| (bseg.uuid, bseg.segment.start, bseg.segment.length))
            .collect::<Vec<_>>()
    }
}

/// Append the second list of BlkDevSegments to the first, or if the last
/// segment of the first argument is adjacent to the first segment of the
/// second argument, merge those two together.
/// Postcondition: left.len() + right.len() - 1 <= result.len()
/// Postcondition: result.len() <= left.len() + right.len()
// FIXME: There is a method that duplicates this algorithm called
// coalesce_segs. These methods should either be unified into a single method
// OR one should go away entirely in solution to:
// https://github.com/stratis-storage/stratisd/issues/762.
pub fn coalesce_blkdevsegs(left: &[BlkDevSegment], right: &[BlkDevSegment]) -> Vec<BlkDevSegment> {
    if left.is_empty() {
        return right.to_vec();
    }
    if right.is_empty() {
        return left.to_vec();
    }

    let mut segments = Vec::with_capacity(left.len() + right.len());
    segments.extend_from_slice(left);

    // Last existing and first new may be contiguous.
    let coalesced = {
        let right_first = right.first().expect("!right.is_empty()");
        let left_last = segments.last_mut().expect("!left.is_empty()");
        if left_last.uuid == right_first.uuid
            && (left_last.segment.start + left_last.segment.length == right_first.segment.start)
        {
            left_last.segment.length += right_first.segment.length;
            true
        } else {
            false
        }
    };

    if coalesced {
        segments.extend_from_slice(&right[1..]);
    } else {
        segments.extend_from_slice(right);
    }
    segments
}

/// Build a linear dev target table from BlkDevSegments. This is useful for
/// calls to the devicemapper library.
pub fn map_to_dm(bsegs: &[BlkDevSegment]) -> Vec<TargetLine<LinearDevTargetParams>> {
    let mut table = Vec::new();
    let mut logical_start_offset = Sectors(0);

    let segments = bsegs
        .into_iter()
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
    pool_uuid: PoolUuid,
    block_devs: Vec<StratBlockDev>,
    last_update_time: Option<DateTime<Utc>>,
}

impl BlockDevMgr {
    /// Make a struct that represents an existing BlockDevMgr.
    pub fn new(
        pool_uuid: PoolUuid,
        block_devs: Vec<StratBlockDev>,
        last_update_time: Option<DateTime<Utc>>,
    ) -> BlockDevMgr {
        BlockDevMgr {
            pool_uuid,
            block_devs,
            last_update_time,
        }
    }

    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(
        pool_uuid: PoolUuid,
        paths: &[&Path],
        mda_size: Sectors,
        force: bool,
    ) -> StratisResult<BlockDevMgr> {
        let devices = resolve_devices(paths)?;
        Ok(BlockDevMgr::new(
            pool_uuid,
            initialize(pool_uuid, devices, mda_size, force, &HashSet::new())?,
            None,
        ))
    }

    /// Return the UUID of the pool
    pub fn pool_uuid(&self) -> PoolUuid {
        self.pool_uuid
    }

    /// Get a function that maps UUIDs to Devices.
    pub fn uuid_to_devno(&self) -> Box<Fn(DevUuid) -> Option<Device>> {
        let uuid_map: HashMap<DevUuid, Device> = self.block_devs
            .iter()
            .map(|bd| (bd.uuid(), *bd.device()))
            .collect();

        Box::new(move |uuid: DevUuid| -> Option<Device> { uuid_map.get(&uuid).cloned() })
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(&mut self, paths: &[&Path], force: bool) -> StratisResult<Vec<DevUuid>> {
        let devices = resolve_devices(paths)?;
        let current_uuids = self.block_devs.iter().map(|bd| bd.uuid()).collect();
        let bds = initialize(
            self.pool_uuid,
            devices,
            MIN_MDA_SECTORS,
            force,
            &current_uuids,
        )?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_uuids)
    }

    pub fn destroy_all(self) -> StratisResult<()> {
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

        let data_size = Bytes(metadata.len() as u64).sectors();
        let candidates = self.block_devs
            .iter_mut()
            .filter(|b| b.max_metadata_size() >= data_size);

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

    /// The current capacity of all the blockdevs.
    /// self.current_capacity() > self.avail_space() because some sectors
    /// are certainly allocated for Stratis metadata
    pub fn current_capacity(&self) -> Sectors {
        self.block_devs.iter().map(|b| b.current_capacity()).sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.current_capacity() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs.iter().map(|bd| bd.metadata_size()).sum()
    }
}

impl Recordable<Vec<BlockDevSave>> for BlockDevMgr {
    fn record(&self) -> Vec<BlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
fn initialize(
    pool_uuid: PoolUuid,
    devices: HashMap<Device, &Path>,
    mda_size: Sectors,
    force: bool,
    owned_devs: &HashSet<DevUuid>,
) -> StratisResult<Vec<StratBlockDev>> {
    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    fn dev_info(devnode: &Path) -> StratisResult<(&Path, Bytes, DevOwnership, File)> {
        let mut f = OpenOptions::new().read(true).write(true).open(&devnode)?;
        let dev_size = blkdev_size(&f)?;
        let ownership = StaticHeader::determine_ownership(&mut f)?;

        Ok((devnode, dev_size, ownership, f))
    }

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    #[allow(type_complexity)]
    fn filter_devs<'a, I>(
        dev_infos: I,
        pool_uuid: PoolUuid,
        force: bool,
        owned_devs: &HashSet<DevUuid>,
    ) -> StratisResult<Vec<(Device, (&'a Path, Bytes, File))>>
    where
        I: Iterator<Item = (Device, StratisResult<(&'a Path, Bytes, DevOwnership, File)>)>,
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            let (devnode, dev_size, ownership, f) = dev_result?;
            if dev_size < MIN_DEV_SIZE {
                let error_message = format!(
                    "{} too small, minimum {} bytes",
                    devnode.display(),
                    MIN_DEV_SIZE
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            };
            match ownership {
                DevOwnership::Unowned => add_devs.push((dev, (devnode, dev_size, f))),
                DevOwnership::Theirs => {
                    if !force {
                        let err_str = format!(
                            "Device {} appears to belong to another application",
                            devnode.display()
                        );
                        return Err(StratisError::Engine(ErrorEnum::Invalid, err_str));
                    } else {
                        add_devs.push((dev, (devnode, dev_size, f)))
                    }
                }
                DevOwnership::Ours(uuid, dev_uuid) => {
                    if pool_uuid == uuid {
                        if !owned_devs.contains(&dev_uuid) {
                            let error_str = format!(
                                "Device {} with pool UUID is unknown to pool",
                                devnode.display()
                            );
                            return Err(StratisError::Engine(ErrorEnum::Invalid, error_str));
                        }
                    } else {
                        let error_str = format!(
                            "Device {} already belongs to Stratis pool {}",
                            devnode.display(),
                            uuid
                        );
                        return Err(StratisError::Engine(ErrorEnum::Invalid, error_str));
                    }
                }
            }
        }
        Ok(add_devs)
    }

    validate_mda_size(mda_size)?;

    let dev_infos = devices.into_iter().map(|(d, p)| (d, dev_info(p)));

    let add_devs = filter_devs(dev_infos, pool_uuid, force, owned_devs)?;

    let mut bds: Vec<StratBlockDev> = Vec::new();
    for (dev, (devnode, dev_size, mut f)) in add_devs {
        let bda = BDA::initialize(
            &mut f,
            pool_uuid,
            Uuid::new_v4(),
            mda_size,
            dev_size.sectors(),
            Utc::now().timestamp() as u64,
        );
        if let Ok(bda) = bda {
            let hw_id = match hw_lookup(devnode) {
                Ok(id) => id,
                Err(_) => None, // TODO: Log this failure so that it can be addressed.
            };

            // FIXME: The expect is only provisionally true.
            // The dev_size is at least MIN_DEV_SIZE, but the size of the
            // metadata is not really bounded from above.
            let blockdev = StratBlockDev::new(dev, devnode.to_owned(), bda, &[], None, hw_id)
                .expect("bda.size() == dev_size; only allocating space for metadata");
            bds.push(blockdev);
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = BDA::wipe(&mut f);
            let _ = wipe_blockdevs(&bds);

            return Err(bda.unwrap_err());
        }
    }
    Ok(bds)
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use rand;
    use uuid::Uuid;

    use devicemapper::SECTOR_SIZE;

    use super::super::super::device::write_sectors;
    use super::super::super::tests::{loopbacked, real};

    use super::super::metadata::{BDA_STATIC_HDR_SECTORS, MIN_MDA_SECTORS};
    use super::super::setup::{find_all, get_metadata};

    use super::*;

    /// Verify that initially,
    /// current_capacity() - metadata_size() = avail_space().
    /// After 2 Sectors have been allocated, that amount must also be included
    /// in balance.
    fn test_blockdevmgr_used(paths: &[&Path]) -> () {
        let mut mgr =
            BlockDevMgr::initialize(Uuid::new_v4(), paths, MIN_MDA_SECTORS, false).unwrap();
        assert_eq!(
            mgr.avail_space() + mgr.metadata_size(),
            mgr.current_capacity()
        );

        let allocated = Sectors(2);
        mgr.alloc_space(&[allocated]).unwrap();
        assert_eq!(
            mgr.avail_space() + allocated + mgr.metadata_size(),
            mgr.current_capacity()
        );
    }

    #[test]
    pub fn loop_test_blockdevmgr_used() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_blockdevmgr_used,
        );
    }

    #[test]
    pub fn real_test_blockdevmgr_used() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_blockdevmgr_used,
        );
    }

    #[test]
    pub fn travis_test_blockdevmgr_used() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_blockdevmgr_used,
        );
    }

    /// Verify that it is impossible to initialize a set of disks of which
    /// even one is dirty, i.e, has some data written within BDA_STATIC_HDR_SECTORS
    /// of start of disk. Choose the dirty disk randomly. This means that even
    /// if our code is broken with respect to this property, this test might
    /// sometimes succeed.
    /// FIXME: Consider enriching device specs so that this test will fail
    /// consistently.
    /// Verify that force flag allows all dirty disks to be initialized.
    fn test_force_flag_dirty(paths: &[&Path]) -> () {
        let index = rand::random::<u8>() as usize % paths.len();
        write_sectors(
            paths[index],
            Sectors(index as u64 % *BDA_STATIC_HDR_SECTORS),
            Sectors(1),
            &[1u8; SECTOR_SIZE],
        ).unwrap();

        let pool_uuid = Uuid::new_v4();
        assert!(BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).is_err());
        assert!(paths.iter().enumerate().all(|(i, path)| {
            StaticHeader::determine_ownership(&mut OpenOptions::new()
                .read(true)
                .open(path)
                .unwrap())
                .unwrap() == if i == index {
                DevOwnership::Theirs
            } else {
                DevOwnership::Unowned
            }
        }));

        assert!(BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, true).is_ok());
        assert!(paths.iter().all(|path| {
            match StaticHeader::determine_ownership(&mut OpenOptions::new()
                .read(true)
                .open(path)
                .unwrap())
                .unwrap()
            {
                DevOwnership::Ours(uuid, _) => pool_uuid == uuid,
                _ => false,
            }
        }));
    }

    #[test]
    pub fn loop_test_force_flag_dirty() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_force_flag_dirty,
        );
    }

    #[test]
    pub fn real_test_force_flag_dirty() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(1, None, None),
            test_force_flag_dirty,
        );
    }

    #[test]
    pub fn travis_test_force_flag_dirty() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(1, 3, None),
            test_force_flag_dirty,
        );
    }

    /// Verify that it is impossible to steal blockdevs from another Stratis
    /// pool.
    /// 1. Initialize devices with pool uuid.
    /// 2. Initializing again with different uuid must fail.
    /// 3. Adding the devices must succeed, because they already belong.
    /// 4. Initializing again with different uuid and force = true also fails.
    fn test_force_flag_stratis(paths: &[&Path]) -> () {
        assert!(paths.len() > 1);
        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let uuid = Uuid::new_v4();
        let uuid2 = Uuid::new_v4();

        let mut bd_mgr = BlockDevMgr::initialize(uuid, paths1, MIN_MDA_SECTORS, false).unwrap();
        assert!(BlockDevMgr::initialize(uuid2, paths1, MIN_MDA_SECTORS, false).is_err());
        // FIXME: this should succeed, but currently it fails, to be extra safe.
        // See: https://github.com/stratis-storage/stratisd/pull/292
        assert!(BlockDevMgr::initialize(uuid2, paths1, MIN_MDA_SECTORS, true).is_err());

        let original_length = bd_mgr.block_devs.len();
        assert!(bd_mgr.add(paths1, false).is_ok());
        assert_eq!(bd_mgr.block_devs.len(), original_length);

        BlockDevMgr::initialize(uuid, paths2, MIN_MDA_SECTORS, false).unwrap();
        assert!(bd_mgr.add(paths2, false).is_err());
    }

    #[test]
    pub fn loop_test_force_flag_stratis() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_force_flag_stratis,
        );
    }

    #[test]
    pub fn real_test_force_flag_stratis() {
        real::test_with_spec(
            real::DeviceLimits::AtLeast(2, None, None),
            test_force_flag_stratis,
        );
    }

    #[test]
    pub fn travis_test_force_flag_stratis() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Range(2, 3, None),
            test_force_flag_stratis,
        );
    }

    /// Verify that find_all function locates and assigns pools appropriately.
    /// 1. Split available paths into 2 discrete sets.
    /// 2. Initialize the block devices in the first set with a pool uuid.
    /// 3. Run find_all() and verify that it has found the initialized devices
    /// and no others.
    /// 4. Initialize the block devices in the second set with a different pool
    /// uuid.
    /// 5. Run find_all() again and verify that both sets of devices are found.
    /// 6. Verify that get_metadata() return an error. initialize() only
    /// initializes block devices, it does not write metadata.
    fn test_initialize(paths: &[&Path]) -> () {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let uuid1 = Uuid::new_v4();
        BlockDevMgr::initialize(uuid1, paths1, MIN_MDA_SECTORS, false).unwrap();

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 1);
        assert!(pools.contains_key(&uuid1));
        let devices = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert_eq!(devices.len(), paths1.len());

        let uuid2 = Uuid::new_v4();
        BlockDevMgr::initialize(uuid2, paths2, MIN_MDA_SECTORS, false).unwrap();

        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);

        assert!(pools.contains_key(&uuid1));
        let devices1 = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert_eq!(devices1.len(), paths1.len());

        assert!(pools.contains_key(&uuid2));
        let devices2 = pools.get(&uuid2).expect("pools.contains_key() was true");
        assert_eq!(devices2.len(), paths2.len());

        assert!(
            pools
                .iter()
                .map(|(uuid, devs)| get_metadata(*uuid, devs))
                .all(|x| x.unwrap().is_none())
        );
    }

    #[test]
    pub fn loop_test_initialize() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_initialize);
    }

    #[test]
    pub fn real_test_initialize() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2, None, None), test_initialize);
    }

    #[test]
    pub fn travis_test_initialize() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3, None), test_initialize);
    }

    /// Test that initialing devices claims all and that destroying
    /// them releases all.
    fn test_ownership(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let bd_mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();

        assert!(paths.iter().all(|path| {
            match StaticHeader::determine_ownership(&mut OpenOptions::new()
                .read(true)
                .open(path)
                .unwrap())
                .unwrap()
            {
                DevOwnership::Ours(uuid, _) => uuid == pool_uuid,
                _ => false,
            }
        }));
        bd_mgr.destroy_all().unwrap();
        assert!(paths.iter().all(|path| {
            StaticHeader::determine_ownership(&mut OpenOptions::new()
                .read(true)
                .open(path)
                .unwrap())
                .unwrap() == DevOwnership::Unowned
        }));
    }

    #[test]
    pub fn loop_test_ownership() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }

    #[test]
    pub fn real_test_ownership() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1, None, None), test_ownership);
    }

    #[test]
    pub fn travis_test_ownership() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }
}
