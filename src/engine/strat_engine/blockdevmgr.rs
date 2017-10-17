// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::collections::{HashMap, HashSet};
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

#[derive(Clone, Debug)]
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
    pool_uuid: PoolUuid,
    block_devs: HashMap<DevUuid, StratBlockDev>,
    last_update_time: Option<DateTime<Utc>>,
}

impl BlockDevMgr {
    pub fn new(pool_uuid: PoolUuid, block_devs: Vec<StratBlockDev>) -> BlockDevMgr {
        BlockDevMgr {
            pool_uuid: pool_uuid,
            block_devs: block_devs
                .into_iter()
                .map(|bd| (bd.uuid(), bd))
                .collect(),
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
        Ok(BlockDevMgr::new(pool_uuid,
                            initialize(pool_uuid, devices, mda_size, force, &HashSet::new())?))
    }

    /// Get a function that maps UUIDs to Devices.
    pub fn uuid_to_devno(&self) -> Box<Fn(DevUuid) -> Option<Device>> {
        let uuid_map: HashMap<DevUuid, Device> = self.block_devs
            .iter()
            .map(|(uuid, bd)| (*uuid, *bd.device()))
            .collect();

        Box::new(move |uuid: DevUuid| -> Option<Device> { uuid_map.get(&uuid).cloned() })
    }

    pub fn add(&mut self, paths: &[&Path], force: bool) -> EngineResult<Vec<DevUuid>> {
        let devices = resolve_devices(paths)?;
        let current_uuids = self.block_devs.keys().cloned().collect();
        let bds = initialize(self.pool_uuid,
                             devices,
                             MIN_MDA_SECTORS,
                             force,
                             &current_uuids)?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs
            .extend(bds.into_iter().map(|bd| (bd.uuid(), bd)));
        Ok(bdev_uuids)
    }

    pub fn destroy_all(mut self) -> EngineResult<()> {
        let bds = self.block_devs
            .drain()
            .map(|(_, bd)| bd)
            .collect::<Vec<_>>();
        wipe_blockdevs(&bds)
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
            for bd in self.block_devs.values_mut() {
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

    #[allow(dead_code)]
    pub fn devnodes(&self) -> Vec<PathBuf> {
        self.block_devs
            .values()
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
            .values_mut()
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
            .values()
            .map(|bd| bd as &BlockDev)
            .collect()
    }

    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&BlockDev> {
        self.block_devs.get(&uuid).map(|bd| bd as &BlockDev)
    }

    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut BlockDev> {
        self.block_devs
            .get_mut(&uuid)
            .map(|bd| bd as &mut BlockDev)
    }

    // SIZE methods

    /// The number of sectors not allocated for any purpose.
    pub fn avail_space(&self) -> Sectors {
        self.block_devs.values().map(|bd| bd.available()).sum()
    }

    /// The current capacity of all the blockdevs.
    /// self.current_capacity() > self.avail_space() because some sectors
    /// are certainly allocated for Stratis metadata
    pub fn current_capacity(&self) -> Sectors {
        self.block_devs
            .values()
            .map(|b| b.current_capacity())
            .sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.current_capacity() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs
            .values()
            .map(|bd| bd.metadata_size())
            .sum()
    }
}

impl Recordable<HashMap<DevUuid, BlockDevSave>> for BlockDevMgr {
    fn record(&self) -> HashMap<Uuid, BlockDevSave> {
        self.block_devs
            .iter()
            .map(|(uuid, bd)| (*uuid, bd.record()))
            .collect()
    }
}

/// Initialize multiple blockdevs at once. This allows all of them
/// to be checked for usability before writing to any of them.
fn initialize(pool_uuid: PoolUuid,
              devices: HashMap<Device, &Path>,
              mda_size: Sectors,
              force: bool,
              owned_devs: &HashSet<DevUuid>)
              -> EngineResult<Vec<StratBlockDev>> {

    /// Get device information, returns an error if problem with obtaining
    /// that information.
    /// Returns a tuple with the device's path, its size in bytes,
    /// its ownership as determined by calling determine_ownership(),
    /// and an open File handle, all of which are needed later.
    fn dev_info(devnode: &Path) -> EngineResult<(&Path, Bytes, DevOwnership, File)> {
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
                          force: bool,
                          owned_devs: &HashSet<DevUuid>)
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
                DevOwnership::Ours(uuid, dev_uuid) => {
                    if pool_uuid == uuid {
                        if !owned_devs.contains(&dev_uuid) {
                            let error_str = format!("Device {} with pool UUID is unknown to pool",
                                                    devnode.display());
                            return Err(EngineError::Engine(ErrorEnum::Invalid, error_str));
                        }
                    } else {
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

    validate_mda_size(mda_size)?;

    let dev_infos = devices.into_iter().map(|(d, p)| (d, dev_info(p)));

    let add_devs = filter_devs(dev_infos, pool_uuid, force, owned_devs)?;

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

            // TODO: support getting hw info and passing in here. See #615
            bds.push(StratBlockDev::new(dev, devnode.to_owned(), bda, allocator, None, None));
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
    use std::io::Write;

    use nix::mount::{MNT_DETACH, MsFlags, mount, umount2};
    use rand;
    use uuid::Uuid;


    use devicemapper::{DM, DataBlocks, DmDevice, DmName, LinearDev, SECTOR_SIZE, ThinDev,
                       ThinDevId, ThinPoolDev};

    use super::super::device::{wipe_sectors, write_sectors};
    use super::super::metadata::{BDA_STATIC_HDR_SECTORS, MIN_MDA_SECTORS};
    use super::super::setup::{find_all, get_metadata};
    use super::super::tests::{loopbacked, real};
    use super::super::tests::tempdir::TempDir;
    use super::super::util::create_fs;

    use super::*;

    /// Verify that initially,
    /// current_capacity() - metadata_size() = avail_space().
    /// After 2 Sectors have been allocated, that amount must also be included
    /// in balance.
    fn test_blockdevmgr_used(paths: &[&Path]) -> () {
        let mut mgr = BlockDevMgr::initialize(Uuid::new_v4(), paths, MIN_MDA_SECTORS, false)
            .unwrap();
        assert_eq!(mgr.avail_space() + mgr.metadata_size(),
                   mgr.current_capacity());

        let allocated = Sectors(2);
        mgr.alloc_space(&[allocated]).unwrap();
        assert_eq!(mgr.avail_space() + allocated + mgr.metadata_size(),
                   mgr.current_capacity());
    }

    #[test]
    pub fn loop_test_blockdevmgr_used() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_blockdevmgr_used);
    }

    #[test]
    pub fn real_test_blockdevmgr_used() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_blockdevmgr_used);
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
        write_sectors(paths[index],
                      Sectors(index as u64 % *BDA_STATIC_HDR_SECTORS),
                      Sectors(1),
                      &[1u8; SECTOR_SIZE])
                .unwrap();

        let pool_uuid = Uuid::new_v4();
        assert!(BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).is_err());
        assert!(paths
                    .iter()
                    .enumerate()
                    .all(|(i, path)| {
            StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                       .read(true)
                                                       .open(path)
                                                       .unwrap())
                    .unwrap() ==
            if i == index {
                DevOwnership::Theirs
            } else {
                DevOwnership::Unowned
            }
        }));

        assert!(BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, true).is_ok());
        assert!(paths
                    .iter()
                    .all(|path| {
            match StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                             .read(true)
                                                             .open(path)
                                                             .unwrap())
                          .unwrap() {
                DevOwnership::Ours(uuid, _) => pool_uuid == uuid,
                _ => false,
            }
        }));
    }

    #[test]
    pub fn loop_test_force_flag_dirty() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_force_flag_dirty);
    }

    #[test]
    pub fn real_test_force_flag_dirty() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_force_flag_dirty);
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
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3),
                                   test_force_flag_stratis);
    }

    #[test]
    pub fn real_test_force_flag_stratis() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2), test_force_flag_stratis);
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
        assert!(pools.len() == 1);
        assert!(pools.contains_key(&uuid1));
        let devices = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert!(devices.len() == paths1.len());

        let uuid2 = Uuid::new_v4();
        BlockDevMgr::initialize(uuid2, paths2, MIN_MDA_SECTORS, false).unwrap();

        let pools = find_all().unwrap();
        assert!(pools.len() == 2);

        assert!(pools.contains_key(&uuid1));
        let devices1 = pools.get(&uuid1).expect("pools.contains_key() was true");
        assert!(devices1.len() == paths1.len());

        assert!(pools.contains_key(&uuid2));
        let devices2 = pools.get(&uuid2).expect("pools.contains_key() was true");
        assert!(devices2.len() == paths2.len());

        assert!(pools
                    .iter()
                    .map(|(uuid, devs)| get_metadata(*uuid, devs))
                    .all(|x| x.unwrap().is_none()));
    }

    #[test]
    pub fn loop_test_initialize() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_initialize);
    }

    #[test]
    pub fn real_test_initialize() {
        real::test_with_spec(real::DeviceLimits::AtLeast(2), test_initialize);
    }

    /// Test that initialing devices claims all and that destroying
    /// them releases all.
    fn test_ownership(paths: &[&Path]) -> () {
        let pool_uuid = Uuid::new_v4();
        let bd_mgr = BlockDevMgr::initialize(pool_uuid, paths, MIN_MDA_SECTORS, false).unwrap();

        assert!(paths
                    .iter()
                    .all(|path| {
            match StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                             .read(true)
                                                             .open(path)
                                                             .unwrap())
                          .unwrap() {
                DevOwnership::Ours(uuid, _) => uuid == pool_uuid,
                _ => false,
            }
        }));
        bd_mgr.destroy_all().unwrap();
        assert!(paths
                    .iter()
                    .all(|path| {
                             StaticHeader::determine_ownership(&mut OpenOptions::new()
                                                                        .read(true)
                                                                        .open(path)
                                                                        .unwrap())
                                     .unwrap() == DevOwnership::Unowned
                         }));
    }

    #[test]
    pub fn loop_test_ownership() {
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(1, 3), test_ownership);
    }

    #[test]
    pub fn real_test_ownership() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_ownership);
    }

    /// Verify no errors when writing files to a mounted filesystem on a thin
    /// device.
    fn test_thinpool_device(paths: &[&Path]) -> () {
        let mut bd_mgr = BlockDevMgr::initialize(Uuid::new_v4(), paths, MIN_MDA_SECTORS, false)
            .unwrap();

        let dm = DM::new().unwrap();

        let mut seg_list = bd_mgr
            .alloc_space(&[Bytes(16 * IEC::Mi).sectors(), Bytes(IEC::Gi).sectors()])
            .unwrap();
        let data_segs = seg_list.pop().expect("len(seg_list) == 2");
        let meta_segs = seg_list.pop().expect("len(seg_list) == 1");

        let metadata_dev = LinearDev::setup(&dm,
                                            DmName::new("stratis_testing_thinpool_metadata")
                                                .expect("valid format"),
                                            None,
                                            &map_to_dm(&meta_segs))
                .unwrap();

        // Clear the meta data device.  If the first block is not all zeros - the
        // stale data will cause the device to appear as an existing meta rather
        // than a new one.  Clear the entire device to be safe.  Stratis implements
        // the same approach when constructing a thin pool.
        wipe_sectors(&metadata_dev.devnode(), Sectors(0), metadata_dev.size()).unwrap();

        let data_dev =
            LinearDev::setup(&dm,
                             DmName::new("stratis_testing_thinpool_datadev").expect("valid format"),
                             None,
                             &map_to_dm(&data_segs))
                    .unwrap();
        let thinpool_dev =
            ThinPoolDev::new(&dm,
                             DmName::new("stratis_testing_thinpool").expect("valid format"),
                             None,
                             Sectors(1024),
                             DataBlocks(256000),
                             metadata_dev,
                             data_dev)
                    .unwrap();
        let thin_dev = ThinDev::new(&dm,
                                    DmName::new("stratis_testing_thindev").expect("valid format"),
                                    None,
                                    &thinpool_dev,
                                    ThinDevId::new_u64(7).expect("7 is small enough"),
                                    Sectors(300000))
                .unwrap();

        create_fs(&thin_dev.devnode(), Uuid::new_v4()).unwrap();

        let tmp_dir = TempDir::new("stratis_testing").unwrap();
        mount(Some(&thin_dev.devnode()),
              tmp_dir.path(),
              Some("xfs"),
              MsFlags::empty(),
              None as Option<&str>)
                .unwrap();
        for i in 0..100 {
            let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
            writeln!(&OpenOptions::new()
                          .create(true)
                          .write(true)
                          .open(file_path)
                          .unwrap(),
                     "data")
                    .unwrap();
        }
        // The MNT_DETACH flags is passed for both loopback and real devs,
        // it helps with loopback devs and does no harm for real devs.
        umount2(tmp_dir.path(), MNT_DETACH).unwrap();
        thin_dev.teardown(&dm).unwrap();
        thinpool_dev.teardown(&dm).unwrap();
    }

    #[test]
    pub fn loop_test_thinpool_device() {
        // This test requires more than 1 GiB.
        loopbacked::test_with_spec(loopbacked::DeviceLimits::Range(2, 3), test_thinpool_device);
    }

    #[test]
    pub fn real_test_thinpool_device() {
        real::test_with_spec(real::DeviceLimits::AtLeast(1), test_thinpool_device);
    }

}
