// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Duration, Utc};
use rand::{seq, thread_rng};
use uuid::Uuid;

use devicemapper::{
    Bytes, Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine, IEC,
};

use crate::{
    engine::{
        engine::BlockDev,
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                device::{resolve_devices, DevOwnership},
                metadata::{disown_device, BlockdevSize, MDADataSize, BDA},
            },
            device::blkdev_size,
            serde_structs::{BaseBlockDevSave, BaseDevSave, Recordable},
            udev::{block_device_apply, decide_ownership, get_udev_property},
        },
        types::{DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

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
    keyfile_path: Option<PathBuf>,
}

impl BlockDevMgr {
    /// Make a struct that represents an existing BlockDevMgr.
    pub fn new(
        block_devs: Vec<StratBlockDev>,
        last_update_time: Option<DateTime<Utc>>,
        keyfile_path: Option<PathBuf>,
    ) -> BlockDevMgr {
        BlockDevMgr {
            block_devs,
            last_update_time,
            keyfile_path,
        }
    }

    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(
        pool_uuid: PoolUuid,
        paths: &[&Path],
        mda_data_size: MDADataSize,
        keyfile_path: Option<PathBuf>,
    ) -> StratisResult<BlockDevMgr> {
        let devices = resolve_devices(paths)?;
        Ok(BlockDevMgr::new(
            initialize(
                pool_uuid,
                devices,
                mda_data_size,
                &HashSet::new(),
                keyfile_path.as_deref(),
            )?,
            None,
            keyfile_path,
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
        let devices = resolve_devices(paths)?;
        let current_uuids = self.block_devs.iter().map(|bd| bd.uuid()).collect();
        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize(
            pool_uuid,
            devices,
            MDADataSize::default(),
            &current_uuids,
            self.keyfile_path.as_deref(),
        )?;
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

    pub fn keyfile_path(&self) -> Option<&Path> {
        self.keyfile_path.as_deref()
    }

    pub fn is_encrypted(&self) -> bool {
        self.keyfile_path.is_some()
    }
}

impl Recordable<Vec<BaseBlockDevSave>> for BlockDevMgr {
    fn record(&self) -> Vec<BaseBlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
fn initialize(
    pool_uuid: PoolUuid,
    devices: HashMap<Device, &Path>,
    mda_data_size: MDADataSize,
    owned_devs: &HashSet<DevUuid>,
    keyfile_path: Option<&Path>,
) -> StratisResult<Vec<StratBlockDev>> {
    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its DevOwnership classification, its optional hw_id,
    /// and an open File handle.
    #[allow(clippy::type_complexity)]
    fn dev_info(
        devnode: &Path,
    ) -> StratisResult<(
        &Path,
        Bytes,
        DevOwnership,
        Option<StratisResult<String>>,
        File,
    )> {
        let f = OpenOptions::new().read(true).write(true).open(&devnode)?;
        let dev_size = blkdev_size(&f)?;

        if let Some((ownership, hw_id)) = block_device_apply(devnode, |d| {
            (
                decide_ownership(d)
                    .and_then(|decision| DevOwnership::from_udev_ownership(&decision, devnode)),
                get_udev_property(d, "ID_WWN"),
            )
        })? {
            Ok((devnode, dev_size, ownership?, hw_id, f))
        } else {
            Err(StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "Could not determine ownership of block device {} because it could not be found in the udev database",
                    devnode.display()
                ),
            ))
        }
    }

    /// Filter devices for admission to pool based on dev_infos.
    /// If there is an error finding out the info, return that error.
    /// Also, return an error if a device is not appropriate for this pool.
    #[allow(clippy::type_complexity)]
    fn filter_devs<'a, I>(
        dev_infos: I,
        pool_uuid: PoolUuid,
        owned_devs: &HashSet<DevUuid>,
    ) -> StratisResult<
        Vec<(
            Device,
            (&'a Path, Bytes, Option<StratisResult<String>>, File),
        )>,
    >
    where
        I: Iterator<
            Item = (
                Device,
                StratisResult<(
                    &'a Path,
                    Bytes,
                    DevOwnership,
                    Option<StratisResult<String>>,
                    File,
                )>,
            ),
        >,
    {
        let mut add_devs = Vec::new();
        for (dev, dev_result) in dev_infos {
            let (devnode, dev_size, ownership, hw_id, f) = dev_result?;
            if dev_size < MIN_DEV_SIZE {
                let error_message =
                    format!("{} too small, minimum {}", devnode.display(), MIN_DEV_SIZE);
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            };
            match ownership {
                DevOwnership::Unowned => add_devs.push((dev, (devnode, dev_size, hw_id, f))),
                DevOwnership::Theirs(info) => {
                    let err_str = format!(
                        "Device {} appears to be already claimed by another, reason: {}",
                        devnode.display(),
                        info
                    );
                    return Err(StratisError::Engine(ErrorEnum::Invalid, err_str));
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

    let dev_infos = devices.into_iter().map(|(d, p)| (d, dev_info(p)));

    let add_devs = filter_devs(dev_infos, pool_uuid, owned_devs)?;

    let mut bds: Vec<StratBlockDev> = Vec::new();
    for (dev, (devnode, dev_size, hw_id, mut f)) in add_devs {
        let bda = BDA::initialize(
            &mut f,
            pool_uuid,
            Uuid::new_v4(),
            mda_data_size,
            BlockdevSize::new(dev_size.sectors()),
            Utc::now().timestamp() as u64,
        );
        if let Ok(bda) = bda {
            let hw_id = match hw_id {
                Some(Ok(hw_id)) => Some(hw_id),
                Some(Err(_)) => {
                    warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                          devnode.display(),
                          pool_uuid.to_simple_ref());
                    None
                }
                None => None,
            };

            // FIXME: The expect is only provisionally true.
            // The dev_size is at least MIN_DEV_SIZE, but the size of the
            // metadata is not really bounded from above.
            let blockdev =
                StratBlockDev::new(dev, devnode.to_owned(), bda, &[], None, hw_id, keyfile_path)
                    .expect("bda.size() == dev_size; only allocating space for metadata");
            bds.push(blockdev);
        } else {
            // TODO: check the return values and update state machine on failure
            let _ = disown_device(&mut f);
            let _ = wipe_blockdevs(&bds);

            return Err(bda.unwrap_err());
        }
    }
    Ok(bds)
}

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &[StratBlockDev]) -> StratisResult<()> {
    let unerased_devnodes: Vec<_> = blockdevs
        .iter()
        .filter_map(|bd| match bd.disown() {
            Err(_) => Some(bd.devnode()),
            _ => None,
        })
        .collect();

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let err_msg = format!(
            "Failed to wipe already initialized devnodes: {:?}",
            unerased_devnodes
        );
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use rand;
    use uuid::Uuid;

    use crate::engine::strat_engine::{
        backstore::{find_all, get_metadata},
        cmd,
        tests::{loopbacked, real},
    };

    use crate::engine::strat_engine::backstore::metadata::device_identifiers;

    use super::*;

    /// Verify that initially,
    /// size() - metadata_size() = avail_space().
    /// After 2 Sectors have been allocated, that amount must also be included
    /// in balance.
    fn test_blockdevmgr_used(paths: &[&Path]) {
        let mut mgr =
            BlockDevMgr::initialize(Uuid::new_v4(), paths, MDADataSize::default(), None).unwrap();
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

    /// Verify that it is impossible to initialize a set of disks of which
    /// even one of them has a signature.  Choose the dirty disk randomly.
    fn test_fail_single_signature(paths: &[&Path]) {
        assert!(paths.len() > 1);
        let index = rand::random::<u8>() as usize % paths.len();

        cmd::create_fs(paths[index], None).unwrap();
        cmd::udev_settle().unwrap();

        let pool_uuid = Uuid::new_v4();
        assert_matches!(
            BlockDevMgr::initialize(pool_uuid, paths, MDADataSize::default(), None),
            Err(_)
        );
        for (i, path) in paths.iter().enumerate() {
            if i == index {
                assert_matches!(
                    DevOwnership::from_udev_ownership(
                        &block_device_apply(path, |d| decide_ownership(d))
                            .unwrap()
                            .unwrap()
                            .unwrap(),
                        path
                    )
                    .unwrap(),
                    DevOwnership::Theirs(_)
                );
            } else {
                assert_matches!(
                    DevOwnership::from_udev_ownership(
                        &block_device_apply(path, |d| decide_ownership(d))
                            .unwrap()
                            .unwrap()
                            .unwrap(),
                        path
                    )
                    .unwrap(),
                    DevOwnership::Unowned
                );
            }
        }

        let clean_paths = paths
            .iter()
            .enumerate()
            .filter(|(n, _)| *n != index)
            .map(|(_, v)| *v)
            .collect::<Vec<&Path>>();

        assert_matches!(
            BlockDevMgr::initialize(pool_uuid, &clean_paths, MDADataSize::default(), None),
            Ok(_)
        );
        cmd::udev_settle().unwrap();

        for path in clean_paths {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap()
                    .unwrap()
                    .0
            );
        }
    }

    #[test]
    pub fn loop_test_fail_single_signature() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_fail_single_signature,
        );
    }

    #[test]
    pub fn real_test_fail_single_signature() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_fail_single_signature,
        );
    }

    #[test]
    pub fn travis_test_fail_single_signature() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_fail_single_signature,
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

        let mut bd_mgr =
            BlockDevMgr::initialize(uuid, paths1, MDADataSize::default(), None).unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(
            BlockDevMgr::initialize(uuid2, paths1, MDADataSize::default(), None),
            Err(_)
        );

        let original_length = bd_mgr.block_devs.len();
        assert_matches!(bd_mgr.add(uuid, paths1), Ok(_));
        assert_eq!(bd_mgr.block_devs.len(), original_length);

        BlockDevMgr::initialize(uuid, paths2, MDADataSize::default(), None).unwrap();
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
    fn test_initialize(paths: &[&Path]) {
        assert!(paths.len() > 1);

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let uuid1 = Uuid::new_v4();
        BlockDevMgr::initialize(uuid1, paths1, MDADataSize::default(), None).unwrap();

        cmd::udev_settle().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 1);
        assert!(pools.contains_key(&uuid1));
        let devices = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert_eq!(devices.len(), paths1.len());

        let uuid2 = Uuid::new_v4();
        BlockDevMgr::initialize(uuid2, paths2, MDADataSize::default(), None).unwrap();

        cmd::udev_settle().unwrap();
        let pools = find_all().unwrap();
        assert_eq!(pools.len(), 2);

        assert!(pools.contains_key(&uuid1));
        let devices1 = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert_eq!(devices1.len(), paths1.len());

        assert!(pools.contains_key(&uuid2));
        let devices2 = pools.get(&uuid2).expect("pools.contains_key() was true");
        assert_eq!(devices2.len(), paths2.len());

        assert!(pools
            .iter()
            .map(|(uuid, devs)| get_metadata(*uuid, devs))
            .all(|x| x.unwrap().is_none()));
    }

    #[test]
    pub fn loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialize,
        );
    }

    #[test]
    pub fn real_test_initialize() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(2, None, None), test_initialize);
    }

    #[test]
    pub fn travis_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialize,
        );
    }

    /// Test that initialing devices claims all and that destroying
    /// them releases all.
    fn test_ownership(paths: &[&Path]) {
        let pool_uuid = Uuid::new_v4();
        let mut bd_mgr =
            BlockDevMgr::initialize(pool_uuid, paths, MDADataSize::default(), None).unwrap();

        cmd::udev_settle().unwrap();

        for path in paths {
            assert_eq!(
                pool_uuid,
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap()
                    .unwrap()
                    .0
            );
        }

        bd_mgr.destroy_all().unwrap();

        for path in paths {
            assert_eq!(
                device_identifiers(&mut OpenOptions::new().read(true).open(path).unwrap(),)
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    pub fn loop_test_ownership() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }

    #[test]
    pub fn real_test_ownership() {
        real::test_with_spec(&real::DeviceLimits::AtLeast(1, None, None), test_ownership);
    }

    #[test]
    pub fn travis_test_ownership() {
        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Range(1, 3, None), test_ownership);
    }
}
