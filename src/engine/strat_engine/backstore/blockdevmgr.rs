// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use rand::{seq::IteratorRandom, thread_rng};

use devicemapper::{Bytes, Device, Sectors};

use crate::{
    engine::{
        shared::gather_encryption_info,
        strat_engine::{
            backstore::{
                blockdev::{v1, v2, InternalBlockDev},
                devices::{
                    initialize_devices, initialize_devices_legacy, wipe_blockdevs, UnownedDevices,
                },
                shared::{BlkDevSegment, Segment},
            },
            crypt::handle::v1::CryptHandle,
            metadata::{MDADataSize, BDA},
            serde_structs::{BaseBlockDevSave, Recordable},
            shared::bds_to_bdas,
        },
        types::{DevUuid, EncryptionInfo, Name, PoolEncryptionInfo, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

const MAX_NUM_TO_WRITE: usize = 10;

#[derive(Debug)]
struct TimeStamp {
    inner: Option<DateTime<Utc>>,
}

impl From<Option<DateTime<Utc>>> for TimeStamp {
    fn from(time: Option<DateTime<Utc>>) -> Self {
        TimeStamp { inner: time }
    }
}

impl TimeStamp {
    // Get the best next time stamp; at least now() and at least one
    // nanosecond more than the previous.
    fn next(&self) -> DateTime<Utc> {
        let current_time = Utc::now();
        if Some(current_time) <= self.inner {
            self.inner
                .expect("self.inner >= Some(current_time")
                .checked_add_signed(Duration::nanoseconds(1))
                .expect("self.inner << maximum representable DateTime")
        } else {
            current_time
        }
    }

    // Set the time stamp to a new value.
    fn set(&mut self, time: DateTime<Utc>) {
        self.inner = Some(time);
    }
}

#[derive(Debug)]
pub struct BlockDevMgr<B> {
    /// All the block devices that belong to this block dev manager.
    block_devs: Vec<B>,
    /// The most recent time that variable length metadata was saved to the
    /// devices managed by this block dev manager.
    last_update_time: TimeStamp,
}

impl BlockDevMgr<v1::StratBlockDev> {
    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        mda_data_size: MDADataSize,
        encryption_info: Option<&EncryptionInfo>,
        sector_size: Option<u32>,
    ) -> StratisResult<BlockDevMgr<v1::StratBlockDev>> {
        Ok(BlockDevMgr::new(
            initialize_devices_legacy(
                devices,
                pool_name,
                pool_uuid,
                mda_data_size,
                encryption_info,
                sector_size,
            )?,
            None,
        ))
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(
        &mut self,
        pool_name: Name,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        sector_size: Option<u32>,
    ) -> StratisResult<Vec<DevUuid>> {
        let this_pool_uuid = self.block_devs.first().map(|bd| bd.pool_uuid());
        if this_pool_uuid.is_some() && this_pool_uuid != Some(pool_uuid) {
            return Err(StratisError::Msg(
                format!("block devices being managed have pool UUID {} but new devices are to be added with pool UUID {}",
                        this_pool_uuid.expect("guarded by if-expression"),
                        pool_uuid)
            ));
        }

        let encryption_info = pool_enc_to_enc!(self.encryption_info());
        if let Some(ref ei) = encryption_info {
            if !CryptHandle::can_unlock(
                self.block_devs
                    .first()
                    .expect("Must have at least one blockdev")
                    .physical_path(),
                ei.key_description().is_some(),
                ei.clevis_info().is_some(),
            ) {
                return Err(StratisError::Msg(
                    "Either the key in the kernel keyring, Clevis, or both could not be used to perform encryption operations on the devices in the pool; check that the appropriate key in the keyring is set and that the Clevis key storage method is available depending on your provided unlock methods".to_string(),
                ));
            }
        }

        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize_devices_legacy(
            devices,
            pool_name,
            pool_uuid,
            MDADataSize::default(),
            encryption_info.as_ref(),
            sector_size,
        )?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_uuids)
    }

    /// Get the encryption information for a whole pool.
    pub fn encryption_info(&self) -> Option<PoolEncryptionInfo> {
        gather_encryption_info(
            self.block_devs.len(),
            self.block_devs.iter().map(|bd| bd.encryption_info()),
        )
        .expect("Cannot create a pool out of both encrypted and unencrypted devices")
    }

    pub fn is_encrypted(&self) -> bool {
        self.encryption_info().is_some()
    }

    #[cfg(test)]
    fn invariant(&self) {
        let pool_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.pool_uuid())
            .collect::<HashSet<_>>();
        assert!(pool_uuids.len() == 1);

        let encryption_infos = self
            .block_devs
            .iter()
            .filter_map(|bd| bd.encryption_info())
            .collect::<Vec<_>>();
        if encryption_infos.is_empty() {
            assert_eq!(self.encryption_info(), None);
        } else {
            assert_eq!(encryption_infos.len(), self.block_devs.len());

            let info_set = encryption_infos.iter().collect::<HashSet<_>>();
            assert!(info_set.len() == 1);
        }

        for bd in self.block_devs.iter() {
            bd.invariant();
        }
    }
}

impl BlockDevMgr<v2::StratBlockDev> {
    /// Initialize a new StratBlockDevMgr with specified pool and devices.
    pub fn initialize(
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
        mda_data_size: MDADataSize,
    ) -> StratisResult<BlockDevMgr<v2::StratBlockDev>> {
        Ok(BlockDevMgr::new(
            initialize_devices(devices, pool_uuid, mda_data_size)?,
            None,
        ))
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(
        &mut self,
        pool_uuid: PoolUuid,
        devices: UnownedDevices,
    ) -> StratisResult<Vec<DevUuid>> {
        let this_pool_uuid = self.block_devs.first().map(|bd| bd.pool_uuid());
        if this_pool_uuid.is_some() && this_pool_uuid != Some(pool_uuid) {
            return Err(StratisError::Msg(
                format!("block devices being managed have pool UUID {} but new devices are to be added with pool UUID {}",
                        this_pool_uuid.expect("guarded by if-expression"),
                        pool_uuid)
            ));
        }

        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize_devices(devices, pool_uuid, MDADataSize::default())?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_uuids)
    }

    #[cfg(test)]
    fn invariant(&self) {
        let pool_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.pool_uuid())
            .collect::<HashSet<_>>();
        assert!(pool_uuids.len() == 1);

        for bd in self.block_devs.iter() {
            bd.invariant();
        }
    }
}

impl<B> BlockDevMgr<B>
where
    B: InternalBlockDev,
{
    /// Make a struct that represents an existing BlockDevMgr.
    pub fn new(block_devs: Vec<B>, last_update_time: Option<DateTime<Utc>>) -> BlockDevMgr<B> {
        BlockDevMgr {
            block_devs,
            last_update_time: last_update_time.into(),
        }
    }

    /// Convert the BlockDevMgr into a collection of BDAs.
    pub fn into_bdas(self) -> HashMap<DevUuid, BDA> {
        bds_to_bdas(self.block_devs)
    }

    /// Get a hashmap that maps UUIDs to Devices.
    pub fn uuid_to_devno(&self) -> HashMap<DevUuid, Device> {
        self.block_devs
            .iter()
            .map(|bd| (bd.uuid(), *bd.device()))
            .collect()
    }

    pub fn destroy_all(&mut self) -> StratisResult<()> {
        wipe_blockdevs(&mut self.block_devs)
    }

    /// Drain the BlockDevMgr block devices into a collection of block devices.
    pub fn drain_bds(&mut self) -> Vec<B> {
        self.block_devs.drain(..).collect::<Vec<_>>()
    }

    /// Get references to managed blockdevs.
    pub fn blockdevs(&self) -> Vec<(DevUuid, &B)> {
        self.block_devs.iter().map(|bd| (bd.uuid(), bd)).collect()
    }

    pub fn blockdevs_mut(&mut self) -> Vec<(DevUuid, &mut B)> {
        self.block_devs
            .iter_mut()
            .map(|bd| (bd.uuid(), bd))
            .collect()
    }

    pub fn get_blockdev_by_uuid(&self, uuid: DevUuid) -> Option<&B> {
        self.block_devs.iter().find(|bd| bd.uuid() == uuid)
    }

    pub fn get_mut_blockdev_by_uuid(&mut self, uuid: DevUuid) -> Option<&mut B> {
        self.block_devs.iter_mut().find(|bd| bd.uuid() == uuid)
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
                return Err(StratisError::Msg(format!(
                    "Blockdev corresponding to UUID: {uuid} not found."
                )));
            }
        }
        wipe_blockdevs(&mut removed)?;
        Ok(())
    }

    /// Allocate space according to sizes vector request.
    /// Return the segments allocated for each request, or None if it was
    /// not possible to satisfy the request.
    /// This method is atomic, it either allocates all requested or allocates
    /// nothing.
    pub fn alloc(&mut self, sizes: &[Sectors]) -> Option<Vec<Vec<BlkDevSegment>>> {
        let total_needed: Sectors = sizes.iter().cloned().sum();
        if self.avail_space() < total_needed {
            return None;
        }

        let mut lists = Vec::new();

        for &needed in sizes.iter() {
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

                let r_segs = bd.alloc(needed - alloc);
                let blkdev_segs = r_segs.iter().map(|(&start, &length)| {
                    BlkDevSegment::new(bd.uuid(), Segment::new(*bd.device(), start, length))
                });
                segs.extend(blkdev_segs);
                alloc += r_segs.sum();
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
        let stamp_time = self.last_update_time.next();

        let data_size = Bytes::from(metadata.len());
        let candidates = self
            .block_devs
            .iter_mut()
            .filter(|b| b.max_metadata_size().bytes() >= data_size);

        debug!(
            "Writing {} of pool level metadata to devices in pool",
            data_size
        );

        // TODO: consider making selection not entirely random, i.e, ensuring
        // distribution of metadata over different paths.
        let saved = candidates
            .choose_multiple(&mut thread_rng(), MAX_NUM_TO_WRITE)
            .iter_mut()
            .fold(false, |acc, b| {
                acc | b.save_state(&stamp_time, metadata).is_ok()
            });

        if saved {
            self.last_update_time.set(stamp_time);
            Ok(())
        } else {
            let err_msg = "Failed to save metadata to even one device in pool";
            Err(StratisError::Msg(err_msg.into()))
        }
    }

    /// Load the most recently written metadata from the block devices.
    /// Returns an error if there is some metadata on a device but it could
    /// not be loaded.
    pub fn load_state(&self) -> StratisResult<Vec<u8>> {
        let (mut last_updated, mut result) = (None, None);
        for metadata in self.block_devs.iter().map(|dev| dev.load_state()) {
            match metadata {
                Ok(Some((metadata, updated))) => {
                    if Some(updated) > last_updated {
                        (last_updated, result) = (Some(updated), Some(metadata));
                    }
                }
                Ok(None) => {}
                Err(err) => return Err(err),
            }
        }

        result.ok_or_else(|| {
            StratisError::Msg(
                "No pool-level metadata could be obtained for the specified pool".into(),
            )
        })
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
        self.block_devs
            .iter()
            .map(|b| b.total_size().sectors())
            .sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.allocated_size() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|bd| bd.metadata_size().sectors())
            .sum()
    }

    pub fn grow(&mut self, dev: DevUuid) -> StratisResult<bool> {
        let bd = self
            .block_devs
            .iter_mut()
            .find(|bd| bd.uuid() == dev)
            .ok_or_else(|| StratisError::Msg(format!("Block device with UUID {dev} not found")))?;
        bd.grow()
    }

    /// Tear down devicemapper devices for the block devices in this BlockDevMgr.
    pub fn teardown(&mut self) -> StratisResult<()> {
        let errs = self.block_devs.iter_mut().fold(Vec::new(), |mut errs, bd| {
            if let Err(e) = bd.teardown() {
                errs.push(e);
            }
            errs
        });

        if errs.is_empty() {
            Ok(())
        } else {
            Err(StratisError::BestEffortError("Failed to remove devicemapper devices for some or all physical devices in the pool".to_string(), errs))
        }
    }
}

impl<B> Recordable<Vec<BaseBlockDevSave>> for BlockDevMgr<B>
where
    B: Recordable<BaseBlockDevSave>,
{
    fn record(&self) -> Vec<BaseBlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::engine::{
        strat_engine::{
            backstore::{
                blockdev,
                devices::{ProcessedPathInfos, UnownedDevices},
            },
            cmd,
            tests::{crypt, loopbacked, real},
        },
        types::KeyDescription,
    };

    use super::*;

    fn get_devices(paths: &[&Path]) -> StratisResult<UnownedDevices> {
        ProcessedPathInfos::try_from(paths)
            .map(|ps| ps.unpack())
            .and_then(|(sds, uds)| sds.error_on_not_empty().map(|_| uds))
    }

    mod v1 {
        use super::*;

        /// Verify that initially,
        /// size() - metadata_size() = avail_space().
        /// After 2 Sectors have been allocated, that amount must also be included
        /// in balance.
        fn test_blockdevmgr_used(paths: &[&Path]) {
            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            let devices = get_devices(paths).unwrap();
            let mut mgr = BlockDevMgr::<blockdev::v1::StratBlockDev>::initialize(
                pool_name,
                pool_uuid,
                devices,
                MDADataSize::default(),
                None,
                None,
            )
            .unwrap();
            assert_eq!(mgr.avail_space() + mgr.metadata_size(), mgr.size());

            let allocated = Sectors(2);
            mgr.alloc(&[allocated]).unwrap();
            assert_eq!(
                mgr.avail_space() + allocated + mgr.metadata_size(),
                mgr.size()
            );
        }

        #[test]
        fn loop_test_blockdevmgr_used() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(1, 3, None),
                test_blockdevmgr_used,
            );
        }

        #[test]
        fn real_test_blockdevmgr_used() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(1, None, None),
                test_blockdevmgr_used,
            );
        }

        /// Test that the `BlockDevMgr` will add devices if the same key
        /// is used to encrypted the existing devices and the added devices.
        fn test_blockdevmgr_same_key(paths: &[&Path]) {
            fn test_with_key(paths: &[&Path], key_desc: &KeyDescription) {
                let pool_uuid = PoolUuid::new_v4();

                let devices1 = get_devices(&paths[..2]).unwrap();
                let devices2 = get_devices(&paths[2..3]).unwrap();

                let pool_name = Name::new("pool_name".to_string());
                let mut bdm = BlockDevMgr::<blockdev::v1::StratBlockDev>::initialize(
                    pool_name.clone(),
                    pool_uuid,
                    devices1,
                    MDADataSize::default(),
                    Some(&EncryptionInfo::KeyDesc(key_desc.clone())),
                    None,
                )
                .unwrap();

                if bdm.add(pool_name, pool_uuid, devices2, None).is_err() {
                    panic!(
                        "Adding a blockdev with the same key to an encrypted pool should succeed"
                    )
                }
            }

            crypt::insert_and_cleanup_key(paths, test_with_key);
        }

        #[test]
        fn loop_test_blockdevmgr_same_key() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Exactly(3, None),
                test_blockdevmgr_same_key,
            );
        }

        #[test]
        fn real_test_blockdevmgr_same_key() {
            real::test_with_spec(
                &real::DeviceLimits::Exactly(3, None, None),
                test_blockdevmgr_same_key,
            );
        }

        /// Test that the `BlockDevMgr` will not add devices if a different key
        /// is present in the keyring than was used to encrypted the existing
        /// devices.
        fn test_blockdevmgr_changed_key(paths: &[&Path]) {
            fn test_with_key(paths: &[&Path], key_desc: &KeyDescription) {
                let pool_uuid = PoolUuid::new_v4();

                let devices1 = get_devices(&paths[..2]).unwrap();
                let devices2 = get_devices(&paths[2..3]).unwrap();

                let pool_name = Name::new("pool_name".to_string());
                let mut bdm = BlockDevMgr::<blockdev::v1::StratBlockDev>::initialize(
                    pool_name.clone(),
                    pool_uuid,
                    devices1,
                    MDADataSize::default(),
                    Some(&EncryptionInfo::KeyDesc(key_desc.clone())),
                    None,
                )
                .unwrap();

                crypt::change_key(key_desc);

                if bdm.add(pool_name, pool_uuid, devices2, None).is_ok() {
                    panic!("Adding a blockdev with a new key to an encrypted pool should fail")
                }
            }

            crypt::insert_and_cleanup_key(paths, test_with_key);
        }

        #[test]
        fn loop_test_blockdevmgr_changed_key() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Exactly(3, None),
                test_blockdevmgr_changed_key,
            );
        }

        #[test]
        fn real_test_blockdevmgr_changed_key() {
            real::test_with_spec(
                &real::DeviceLimits::Exactly(3, None, None),
                test_blockdevmgr_changed_key,
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

            let uuid = PoolUuid::new_v4();
            let uuid2 = PoolUuid::new_v4();
            let pool_name1 = Name::new("pool_name1".to_string());
            let pool_name2 = Name::new("pool_name2".to_string());

            let bd_mgr = BlockDevMgr::<blockdev::v1::StratBlockDev>::initialize(
                pool_name1,
                uuid,
                get_devices(paths1).unwrap(),
                MDADataSize::default(),
                None,
                None,
            )
            .unwrap();
            cmd::udev_settle().unwrap();

            assert_matches!(get_devices(paths1), Err(_));

            assert!(ProcessedPathInfos::try_from(paths1)
                .unwrap()
                .unpack()
                .0
                .partition(uuid2)
                .0
                .is_empty());

            assert!(!ProcessedPathInfos::try_from(paths1)
                .unwrap()
                .unpack()
                .0
                .partition(uuid)
                .0
                .is_empty());

            BlockDevMgr::<blockdev::v1::StratBlockDev>::initialize(
                pool_name2,
                uuid,
                get_devices(paths2).unwrap(),
                MDADataSize::default(),
                None,
                None,
            )
            .unwrap();

            cmd::udev_settle().unwrap();

            assert!(!ProcessedPathInfos::try_from(paths2)
                .unwrap()
                .unpack()
                .0
                .partition(uuid)
                .0
                .is_empty());

            bd_mgr.invariant()
        }

        #[test]
        fn loop_test_initialization_stratis() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(2, 3, None),
                test_initialization_add_stratis,
            );
        }

        #[test]
        fn real_test_initialization_stratis() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(2, None, None),
                test_initialization_add_stratis,
            );
        }
    }

    mod v2 {
        use super::*;

        /// Verify that initially,
        /// size() - metadata_size() = avail_space().
        /// After 2 Sectors have been allocated, that amount must also be included
        /// in balance.
        fn test_blockdevmgr_used(paths: &[&Path]) {
            let pool_uuid = PoolUuid::new_v4();
            let devices = get_devices(paths).unwrap();
            let mut mgr = BlockDevMgr::<blockdev::v2::StratBlockDev>::initialize(
                pool_uuid,
                devices,
                MDADataSize::default(),
            )
            .unwrap();
            assert_eq!(mgr.avail_space() + mgr.metadata_size(), mgr.size());

            let allocated = Sectors(2);
            mgr.alloc(&[allocated]).unwrap();
            assert_eq!(
                mgr.avail_space() + allocated + mgr.metadata_size(),
                mgr.size()
            );
        }

        #[test]
        fn loop_test_blockdevmgr_used() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(1, 3, None),
                test_blockdevmgr_used,
            );
        }

        #[test]
        fn real_test_blockdevmgr_used() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(1, None, None),
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

            let uuid = PoolUuid::new_v4();
            let uuid2 = PoolUuid::new_v4();

            let bd_mgr = BlockDevMgr::<blockdev::v2::StratBlockDev>::initialize(
                uuid,
                get_devices(paths1).unwrap(),
                MDADataSize::default(),
            )
            .unwrap();
            cmd::udev_settle().unwrap();

            assert_matches!(get_devices(paths1), Err(_));

            assert!(ProcessedPathInfos::try_from(paths1)
                .unwrap()
                .unpack()
                .0
                .partition(uuid2)
                .0
                .is_empty());

            assert!(!ProcessedPathInfos::try_from(paths1)
                .unwrap()
                .unpack()
                .0
                .partition(uuid)
                .0
                .is_empty());

            BlockDevMgr::<blockdev::v2::StratBlockDev>::initialize(
                uuid,
                get_devices(paths2).unwrap(),
                MDADataSize::default(),
            )
            .unwrap();

            cmd::udev_settle().unwrap();

            assert!(!ProcessedPathInfos::try_from(paths2)
                .unwrap()
                .unpack()
                .0
                .partition(uuid)
                .0
                .is_empty());

            bd_mgr.invariant()
        }

        #[test]
        fn loop_test_initialization_stratis() {
            loopbacked::test_with_spec(
                &loopbacked::DeviceLimits::Range(2, 3, None),
                test_initialization_add_stratis,
            );
        }

        #[test]
        fn real_test_initialization_stratis() {
            real::test_with_spec(
                &real::DeviceLimits::AtLeast(2, None, None),
                test_initialization_add_stratis,
            );
        }
    }
}
