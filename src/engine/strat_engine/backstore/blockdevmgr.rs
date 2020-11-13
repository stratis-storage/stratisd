// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use chrono::{DateTime, Duration, Utc};
use itertools::Itertools;
use rand::{seq::IteratorRandom, thread_rng};
use serde_json::Value;

use devicemapper::{Bytes, Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                crypt::CryptHandle,
                devices::{initialize_devices, process_and_verify_devices, wipe_blockdevs},
            },
            keys::MemoryPrivateFilesystem,
            metadata::MDADataSize,
            names::KeyDescription,
            serde_structs::{BaseBlockDevSave, BaseDevSave, Recordable},
        },
        types::{CreateAction, DeleteAction, DevUuid, PoolUuid},
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

/// Get crypt handles for the slice of blockdevs.
///
/// Postconditions: blockdevs.len() == the len of the Vec result
fn get_crypt_handles(blockdevs: &[StratBlockDev]) -> StratisResult<Vec<CryptHandle>> {
    let mut handles = Vec::new();
    for bd in blockdevs.iter() {
        let path = bd.devnode().physical_path();
        let crypt_handle_opt = CryptHandle::setup(path)?;
        let crypt_handle = crypt_handle_opt.ok_or_else(|| {
            StratisError::Error(format!(
                "Device {} is not an encrypted device",
                path.display(),
            ))
        })?;
        handles.push(crypt_handle);
    }
    Ok(handles)
}

/// Return whether clevis has been enabled for a given pool.
///
/// Returns Ok(Some(_)) containing the clevis info if clevis is enabled.
/// Returns Ok(None) if clevis is not enabled.
/// Returns an error if there are two blockdevs for which clevis info
/// does not match or some blockdevs which have a clevis configuration and
/// some which do not.
fn clevis_enabled(handles: &mut Vec<CryptHandle>) -> StratisResult<Option<(String, Value)>> {
    fn match_infos(
        clevis_info: Option<(String, Value)>,
        clevis_info_next: Option<(String, Value)>,
    ) -> StratisResult<Option<(String, Value)>> {
        match (clevis_info, clevis_info_next) {
            (Some(c), Some(cn)) => {
                if c != cn {
                    Err(StratisError::Error(
                        "Clevis metadata is inconsistent; not all \
                            devices are using the same Clevis configuration"
                            .to_string(),
                    ))
                } else {
                    Ok(Some(c))
                }
            }
            (None, Some(cn)) => Ok(Some(cn)),
            (Some(_), None) => Err(StratisError::Error(
                "Clevis metadata is inconsistent; not all \
                        devices have clevis enabled."
                    .to_string(),
            )),
            (None, None) => Ok(None),
        }
    }

    let mut clevis_info: Option<Option<(String, Value)>> = None;
    for handle in handles.iter_mut() {
        let clevis_info_next = handle.clevis_info()?;
        if let Some(ci) = clevis_info {
            clevis_info = Some(match_infos(ci, clevis_info_next)?);
        } else {
            clevis_info = Some(clevis_info_next);
        }
    }

    Ok(clevis_info.and_then(|ci| ci))
}
#[derive(Debug)]
pub struct BlockDevMgr {
    block_devs: Vec<StratBlockDev>,
    last_update_time: Option<DateTime<Utc>>,
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
        encryption_info: Option<(&KeyDescription, Option<(&str, &Value)>)>,
    ) -> StratisResult<BlockDevMgr> {
        let devices = process_and_verify_devices(pool_uuid, &HashSet::new(), paths)?;

        Ok(BlockDevMgr::new(
            initialize_devices(devices, pool_uuid, mda_data_size, encryption_info)?,
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

    /// Check that the registered key description for these block devices can
    /// unlock at least one of the existing block devices registered.
    /// Precondition: self.block_devs must have at least one device.
    pub fn has_valid_passphrase(&self) -> bool {
        CryptHandle::can_unlock(
            self.block_devs
                .get(0)
                .expect("Must have at least one blockdev")
                .devnode()
                .physical_path(),
        )
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(&mut self, pool_uuid: PoolUuid, paths: &[&Path]) -> StratisResult<Vec<DevUuid>> {
        let this_pool_uuid = self.block_devs.get(0).map(|bd| bd.pool_uuid());
        if this_pool_uuid.is_some() && this_pool_uuid != Some(pool_uuid) {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!("block devices being managed have pool UUID {} but new devices are to be added with pool UUID {}",
                        this_pool_uuid.expect("guarded by if-expression").to_simple_ref(),
                        pool_uuid)
            ));
        }

        let current_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.uuid())
            .collect::<HashSet<_>>();
        let devices = process_and_verify_devices(pool_uuid, &current_uuids, paths)?;

        let is_encrypted = self.is_encrypted();
        if is_encrypted && !self.has_valid_passphrase() {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                "The key associated with the current registered key description \
                was not able to unlock an existing encrypted device; check that \
                the same key is in the keyring that was used to create the encrypted \
                pool"
                    .to_string(),
            ));
        }

        let clevis_info = if is_encrypted {
            clevis_enabled(&mut get_crypt_handles(&self.block_devs)?)?
        } else {
            None
        };

        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize_devices(
            devices,
            pool_uuid,
            MDADataSize::default(),
            match self.key_desc().as_ref() {
                Some(kd) => Some((
                    kd,
                    clevis_info.as_ref().map(|(pin, val)| (pin.as_str(), val)),
                )),
                None => None,
            },
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
        let saved = candidates
            .choose_multiple(&mut thread_rng(), MAX_NUM_TO_WRITE)
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
        self.block_devs
            .iter()
            .map(|b| b.total_size().sectors())
            .sum()
    }

    /// The number of sectors given over to Stratis metadata
    /// self.size() - self.metadata_size() >= self.avail_space()
    pub fn metadata_size(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|bd| bd.metadata_size().sectors())
            .sum()
    }

    pub fn key_desc(&self) -> Option<&KeyDescription> {
        let mut iter = self
            .block_devs
            .iter()
            .filter_map(|bd| bd.key_description())
            .unique();
        let key_desc = iter.next();

        // Liminal device code will note set up a pool with multiple key description
        // values.
        assert!(iter.next().is_none());

        key_desc
    }

    pub fn is_encrypted(&self) -> bool {
        self.key_desc().is_some()
    }

    #[cfg(test)]
    fn invariant(&self) {
        let pool_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.pool_uuid())
            .collect::<HashSet<_>>();
        assert!(pool_uuids.len() == 1);

        let key_descriptions = self
            .block_devs
            .iter()
            .filter_map(|bd| bd.key_description())
            .collect::<Vec<_>>();
        if key_descriptions.is_empty() {
            assert_eq!(self.key_desc(), None);
        } else {
            assert_eq!(key_descriptions.len(), self.block_devs.len());

            let kd_set = key_descriptions.iter().collect::<HashSet<_>>();
            assert!(kd_set.len() == 1);
        }
    }

    pub fn bind_clevis(&self, pin: &str, clevis_info: &Value) -> StratisResult<CreateAction<()>> {
        fn bind_clevis_loop<'a>(
            key_fs: &MemoryPrivateFilesystem,
            rollback_record: &'a mut Vec<CryptHandle>,
            handles: &'a mut Vec<CryptHandle>,
            key_desc: &KeyDescription,
            pin: &str,
            clevis_info: &Value,
        ) -> StratisResult<()> {
            for mut crypt_handle in handles.drain(..) {
                let res = key_fs.key_op(key_desc, |keyfile_path| {
                    crypt_handle
                        .clevis_bind(keyfile_path, pin, clevis_info)
                        .map_err(StratisError::Crypt)
                });
                if res.is_ok() {
                    rollback_record.push(crypt_handle);
                }
                res?;
            }
            Ok(())
        }

        let key_description = match self.key_desc() {
            Some(kd) => kd,
            None => {
                return Err(StratisError::Error(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        let mut crypt_handles = get_crypt_handles(&self.block_devs)?;
        if clevis_enabled(&mut crypt_handles)?.is_some() {
            return Ok(CreateAction::Identity);
        }

        let key_fs = MemoryPrivateFilesystem::new()?;
        let mut rollback_record = Vec::new();
        let result = bind_clevis_loop(
            &key_fs,
            &mut rollback_record,
            &mut crypt_handles,
            key_description,
            pin,
            clevis_info,
        );

        if result.is_err() {
            rollback_record.into_iter().for_each(|mut crypt_dev| {
                if let Err(e) = crypt_dev.clevis_unbind() {
                    warn!(
                        "Failed to unbind device {} from clevis during \
                        rollback: {}",
                        crypt_dev.physical_device_path().display(),
                        e,
                    );
                }
            });
            result?;
        }
        Ok(CreateAction::Created(()))
    }

    pub fn unbind_clevis(&self) -> StratisResult<DeleteAction<()>> {
        if !self.is_encrypted() {
            return Err(StratisError::Error(
                "Requested pool does not appear to be encrypted".to_string(),
            ));
        }

        let mut crypt_handles = get_crypt_handles(&self.block_devs)?;
        if clevis_enabled(&mut crypt_handles)?.is_none() {
            return Ok(DeleteAction::Identity);
        }

        for mut handle in crypt_handles {
            let res = handle.clevis_unbind().map_err(StratisError::Crypt);
            if let Err(ref e) = res {
                warn!(
                    "Failed to unbind from the tang server using clevis: {}. \
                    This operation cannot be rolled back automatically.",
                    e,
                );
            }
            res?
        }
        Ok(DeleteAction::Deleted(()))
    }
}

impl Recordable<Vec<BaseBlockDevSave>> for BlockDevMgr {
    fn record(&self) -> Vec<BaseBlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use uuid::Uuid;

    use crate::engine::strat_engine::{
        cmd,
        tests::{crypt, loopbacked, real},
    };

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

    #[test]
    fn travis_test_blockdevmgr_used() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_blockdevmgr_used,
        );
    }

    /// Test that the `BlockDevMgr` will add devices if the same key
    /// is used to encrypted the existing devices and the added devices.
    fn test_blockdevmgr_same_key(paths: &[&Path]) {
        fn test_with_key(
            paths: &[&Path],
            key_desc: &KeyDescription,
            _: Option<()>,
        ) -> Result<(), Box<dyn Error>> {
            let pool_uuid = Uuid::new_v4();
            let mut bdm = BlockDevMgr::initialize(
                pool_uuid,
                &paths[..2],
                MDADataSize::default(),
                Some((key_desc, None)),
            )?;

            if bdm.add(pool_uuid, &paths[2..3]).is_err() {
                Err(Box::new(StratisError::Error(
                    "Adding a blockdev with the same key to an encrypted pool should succeed"
                        .to_string(),
                )))
            } else {
                Ok(())
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

    #[test]
    fn travis_test_blockdevmgr_same_key() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(3, None),
            test_blockdevmgr_same_key,
        );
    }

    /// Test that the `BlockDevMgr` will not add devices if a different key
    /// is present in the keyring than was used to encrypted the existing
    /// devices.
    fn test_blockdevmgr_changed_key(paths: &[&Path]) {
        fn test_with_first_key(
            paths: &[&Path],
            key_desc: &KeyDescription,
            _: Option<()>,
        ) -> Result<(PoolUuid, BlockDevMgr), Box<dyn Error>> {
            let pool_uuid = Uuid::new_v4();
            let bdm = BlockDevMgr::initialize(
                pool_uuid,
                &paths[..2],
                MDADataSize::default(),
                Some((key_desc, None)),
            )?;
            Ok((pool_uuid, bdm))
        }

        fn test_with_second_key(
            paths: &[&Path],
            _: &KeyDescription,
            data: (PoolUuid, BlockDevMgr),
        ) -> Result<(), Box<dyn Error>> {
            let (pool_uuid, mut bdm) = data;
            if bdm.add(pool_uuid, &paths[2..3]).is_ok() {
                Err(Box::new(StratisError::Error(
                    "Adding a blockdev with a new key to an encrypted pool should fail".to_string(),
                )))
            } else {
                Ok(())
            }
        }

        crypt::insert_and_cleanup_two_keys(paths, test_with_first_key, test_with_second_key);
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

    #[test]
    fn travis_test_blockdevmgr_changed_key() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(3, None),
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
        assert_matches!(bd_mgr.add(uuid2, paths1), Err(_));
        assert_matches!(bd_mgr.add(uuid, paths1), Ok(_));
        assert_eq!(bd_mgr.block_devs.len(), original_length);

        BlockDevMgr::initialize(uuid, paths2, MDADataSize::default(), None).unwrap();
        cmd::udev_settle().unwrap();

        assert_matches!(bd_mgr.add(uuid, paths2), Err(_));

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

    #[test]
    fn travis_test_initialization_stratis() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_initialization_add_stratis,
        );
    }
}
