// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Duration, Utc};
use rand::{seq::IteratorRandom, thread_rng};
use serde_json::Value;
use tempfile::TempDir;

use devicemapper::{Bytes, Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        shared::gather_encryption_info,
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                crypt::{
                    back_up_luks_header, interpret_clevis_config, restore_luks_header,
                    CryptActivationHandle,
                },
                devices::{initialize_devices, process_and_verify_devices, wipe_blockdevs},
                range_alloc::PerDevSegments,
                transaction::RequestTransaction,
            },
            metadata::MDADataSize,
            serde_structs::{BaseBlockDevSave, BaseDevSave, Recordable},
        },
        types::{
            ActionAvailability, DevUuid, EncryptionInfo, KeyDescription, PoolEncryptionInfo,
            PoolUuid,
        },
    },
    stratis::{StratisError, StratisResult},
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
            start,
            length,
            device,
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

#[derive(Debug)]
pub struct BlockDevMgr {
    /// All the block devices that belong to this block dev manager.
    block_devs: Vec<StratBlockDev>,
    /// The most recent time that variable length metadata was saved to the
    /// devices managed by this block dev manager.
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
        encryption_info: Option<&EncryptionInfo>,
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

    /// Check that the registered key description and Clevis information for these
    /// block devices can unlock at least one of the existing block devices registered.
    /// Precondition: self.block_devs must have at least one device.
    pub fn can_unlock(&self, try_unlock_keyring: bool, try_unlock_clevis: bool) -> bool {
        CryptActivationHandle::can_unlock(
            self.block_devs
                .get(0)
                .expect("Must have at least one blockdev")
                .physical_path(),
            try_unlock_keyring,
            try_unlock_clevis,
        )
    }

    /// Add paths to self.
    /// Return the uuids of all blockdevs corresponding to paths that were
    /// added.
    pub fn add(&mut self, pool_uuid: PoolUuid, paths: &[&Path]) -> StratisResult<Vec<DevUuid>> {
        let this_pool_uuid = self.block_devs.get(0).map(|bd| bd.pool_uuid());
        if this_pool_uuid.is_some() && this_pool_uuid != Some(pool_uuid) {
            return Err(StratisError::Msg(
                format!("block devices being managed have pool UUID {} but new devices are to be added with pool UUID {}",
                        this_pool_uuid.expect("guarded by if-expression"),
                        pool_uuid)
            ));
        }

        let current_uuids = self
            .block_devs
            .iter()
            .map(|bd| bd.uuid())
            .collect::<HashSet<_>>();
        let devices = process_and_verify_devices(pool_uuid, &current_uuids, paths)?;

        let encryption_info = pool_enc_to_enc!(self.encryption_info());
        if let Some(ref ei) = encryption_info {
            if !self.can_unlock(ei.key_description().is_some(), ei.clevis_info().is_some()) {
                return Err(StratisError::Msg(
                    "Neither the key in the kernel keyring nor Clevis could be used to perform encryption operations on the devices in the pool; check that either the appropriate key in the keyring is set or that the Clevis key storage method is available".to_string(),
                ));
            }
        }

        // FIXME: This is a bug. If new devices are added to a pool, and the
        // variable length metadata requires more than the minimum allocated,
        // then the necessary amount must be provided or the data can not be
        // saved.
        let bds = initialize_devices(
            devices,
            pool_uuid,
            MDADataSize::default(),
            encryption_info.as_ref(),
        )?;
        let bdev_uuids = bds.iter().map(|bd| bd.uuid()).collect();
        self.block_devs.extend(bds);
        Ok(bdev_uuids)
    }

    pub fn destroy_all(&mut self) -> StratisResult<()> {
        wipe_blockdevs(&mut self.block_devs)
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
                    "Blockdev corresponding to UUID: {} not found.",
                    uuid
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
    pub fn request_space(&self, sizes: &[Sectors]) -> Option<RequestTransaction> {
        let mut transaction = RequestTransaction::default();

        let total_needed: Sectors = sizes.iter().cloned().sum();
        if self.avail_space() < total_needed {
            return None;
        }

        for (idx, &needed) in sizes.iter().enumerate() {
            let mut alloc = Sectors(0);
            // TODO: Consider greater efficiency for allocation generally.
            // Over time, the blockdevs at the start will be exhausted. It
            // might be a good idea to keep an auxiliary structure, so that
            // only blockdevs with some space left to allocate are accessed.
            // In the context of this major inefficiency that ensues over time
            // the obvious but more minor inefficiency of this inner loop is
            // not worth worrying about.
            for bd in &self.block_devs {
                if alloc == needed {
                    break;
                }

                let r_segs = bd.request_space(needed - alloc);
                for (&start, &length) in r_segs.iter() {
                    transaction.add_bd_seg_req(
                        idx,
                        BlkDevSegment::new(bd.uuid(), Segment::new(*bd.device(), start, length)),
                    );
                }
                alloc += r_segs.sum();
            }
            assert_eq!(alloc, needed);
        }

        Some(transaction)
    }

    /// Commit the allocations calculated by the request_space() method.
    ///
    /// This method converts the block device segments into the necessary data
    /// structure and dispatches them to the corresponding block devices to
    /// update the internal records of allocated space.
    pub fn commit_space(&mut self, mut transaction: RequestTransaction) -> StratisResult<()> {
        let mut segs = transaction.drain_blockdevmgr().try_fold(
            HashMap::<DevUuid, PerDevSegments>::new(),
            |mut map, seg| -> StratisResult<_> {
                if let Some(segs) = map.get_mut(&seg.uuid) {
                    segs.insert(&(seg.segment.start, seg.segment.length))?;
                } else {
                    let mut segs = PerDevSegments::new(
                        self.block_devs
                            .iter()
                            .find(|bd| bd.uuid() == seg.uuid)
                            .expect(
                                "Block dev was determined to be present during allocation request",
                            )
                            .total_size()
                            .sectors(),
                    );
                    segs.insert(&(seg.segment.start, seg.segment.length))?;
                    map.insert(seg.uuid, segs);
                }

                Ok(map)
            },
        )?;

        for (uuid, bd) in self.blockdevs_mut() {
            if let Some(segs) = segs.remove(&uuid) {
                bd.commit_space(segs);
            }
        }

        Ok(())
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

        let data_size = Bytes::from(metadata.len());
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
            Err(StratisError::Msg(err_msg.into()))
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

    /// The current size of allocated regions of the blockdevs.
    /// self.allocated_size() > self.avail_space() because some sectors are
    /// allocated for Stratis metadata. Even if the entire block device is not
    /// allocated, the amount of space available for storage will be less than
    /// that of the total amount allocated.
    pub fn allocated_size(&self) -> Sectors {
        self.block_devs
            .iter()
            .map(|b| b.total_allocated_size().sectors())
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
    }

    /// Bind all devices in the given blockdev manager using the given clevis
    /// configuration.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    /// nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    /// or binding failed.
    pub fn bind_clevis(&mut self, pin: &str, clevis_info: &Value) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        let mut parsed_config = clevis_info.clone();
        let yes = interpret_clevis_config(pin, &mut parsed_config)?;

        if let Some((ref existing_pin, ref existing_info)) = encryption_info.clevis_info() {
            // Ignore thumbprint if stratis:tang:trust_url is set in the clevis_info
            // config.
            let mut config_to_check = existing_info.clone();
            if yes {
                if let Value::Object(ref mut ei) = config_to_check {
                    ei.remove("thp");
                }
            }

            if (existing_pin.as_str(), &config_to_check) == (pin, &parsed_config)
                && self.can_unlock(false, true)
            {
                Ok(false)
            } else {
                Err(StratisError::Msg(format!(
                    "Block devices have already been bound with pin {} and config {}; \
                        requested pin {} and config {} can't be applied",
                    existing_pin, existing_info, pin, parsed_config,
                )))
            }
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.bind_clevis(pin, clevis_info),
            )?;
            Ok(true)
        }
    }

    /// Unbind all devices in the given blockdev manager from clevis.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    /// nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    /// or unbinding failed.
    pub fn unbind_clevis(&mut self) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.clevis_info().is_some() {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.unbind_clevis(),
            )?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Bind all devices in the given blockdev manager to a passphrase using the
    /// given key description.
    ///
    /// * Returns Ok(true) if the binding was performed.
    /// * Returns Ok(false) if the binding had already been previously performed and
    /// nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    /// or binding failed.
    pub fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if let Some(kd) = encryption_info.key_description() {
            if kd == key_desc {
                if self.can_unlock(true, false) {
                    Ok(false)
                } else {
                    Err(StratisError::Msg(format!(
                        "Key description {} is registered in the metadata but the \
                            associated passphrase can't unlock the device; the \
                            associated passphrase may have changed since binding",
                        key_desc.as_application_str(),
                    )))
                }
            } else {
                Err(StratisError::Msg(format!(
                    "Block devices have already been bound with key description {}; \
                        requested key description {} can't be applied",
                    key_desc.as_application_str(),
                    kd.as_application_str(),
                )))
            }
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.bind_keyring(key_desc),
            )?;
            Ok(true)
        }
    }

    /// Unbind all devices in the given blockdev manager from the passphrase
    /// associated with the key description.
    ///
    /// * Returns Ok(true) if the unbinding was performed.
    /// * Returns Ok(false) if the unbinding had already been previously performed and
    /// nothing was changed.
    /// * Returns Err(_) if an inconsistency was found in the metadata across pools
    /// or unbinding failed.
    pub fn unbind_keyring(&mut self) -> StratisResult<bool> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.key_description().is_some() {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.unbind_keyring(),
            )?;
            Ok(true)
        } else {
            // is encrypted and key description is None
            Ok(false)
        }
    }

    /// Change the keyring passphrase associated with the block devices in
    /// this pool.
    ///
    /// Returns:
    /// * Ok(None) if the pool is not currently bound to a keyring passphrase.
    /// * Ok(Some(true)) if the pool was successfully bound to the new key description.
    /// * Ok(Some(false)) if the pool is already bound to this key description.
    /// * Err(_) if an operation fails while changing the passphrase.
    pub fn rebind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<Option<bool>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.key_description() == Some(key_desc) {
            Ok(Some(false))
        } else if encryption_info.key_description().is_some() {
            // Keys are not the same but key description is present
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.rebind_keyring(key_desc),
            )?;
            Ok(Some(true))
        } else {
            Ok(None)
        }
    }

    /// Regenerate the Clevis bindings with the block devices in this pool using
    /// the same configuration.
    ///
    /// The method for this rollback caches the initial Clevis metadata and
    /// reverts all of the devices if there is a failure.
    ///
    /// This method returns StratisResult<()> because the Clevis regen command
    /// will always change the metadata when successful. The command is not idempotent
    /// so this method will either fail to regenerate the bindings or it will
    /// result in a metadata change.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ));
            }
        };

        if encryption_info.clevis_info().is_none() {
            Err(StratisError::Msg(
                "Requested pool is not already bound to Clevis".to_string(),
            ))
        } else {
            operation_loop(
                self.blockdevs_mut().into_iter().map(|(_, bd)| bd),
                |blockdev| blockdev.rebind_clevis(),
            )?;

            Ok(())
        }
    }
}

fn operation_loop<'a, I, A>(blockdevs: I, action: A) -> StratisResult<()>
where
    I: IntoIterator<Item = &'a mut StratBlockDev>,
    A: Fn(&mut StratBlockDev) -> StratisResult<()>,
{
    fn rollback_loop(
        rollback_record: Vec<&mut StratBlockDev>,
        headers: Vec<PathBuf>,
        causal_error: StratisError,
    ) -> StratisError {
        // NOTE: Zip can be used here because the header will always be backed up before
        // the operation is performed. As a result, the header iterator will always be
        // equal to or longer than the blockdev record iterator which means all blockdevs
        // that have had operations performed on them will always be restored.
        for (blockdev, header) in rollback_record.into_iter().zip(headers) {
            if let Err(e) = restore_luks_header(blockdev.devnode(), header.as_path()) {
                warn!(
                    "Failed to roll back device operation for device {}: {}",
                    blockdev.physical_path().display(),
                    e,
                );
                return StratisError::RollbackError {
                    causal_error: Box::new(causal_error),
                    rollback_error: Box::new(e),
                    level: ActionAvailability::NoRequests,
                };
            }
        }

        causal_error
    }

    fn perform_operation<'a, I, A>(tmp_dir: &TempDir, blockdevs: I, action: A) -> StratisResult<()>
    where
        I: IntoIterator<Item = &'a mut StratBlockDev>,
        A: Fn(&mut StratBlockDev) -> StratisResult<()>,
    {
        let mut original_headers = Vec::new();
        let mut rollback_record = Vec::new();
        for blockdev in blockdevs {
            match back_up_luks_header(blockdev.physical_path(), tmp_dir) {
                Ok(h) => original_headers.push(h),
                Err(e) => return Err(rollback_loop(rollback_record, original_headers, e)),
            };
            let res = action(blockdev);
            rollback_record.push(blockdev);
            if let Err(error) = res {
                return Err(rollback_loop(rollback_record, original_headers, error));
            }
        }

        Ok(())
    }

    let tmp_dir = TempDir::new()?;
    let res = perform_operation(&tmp_dir, blockdevs, action);
    if let Err(e) = fs::remove_dir_all(tmp_dir.path()) {
        warn!(
            "Leaked temporary files at path {}: {}",
            tmp_dir.path().display(),
            e
        );
    }
    res
}

impl Recordable<Vec<BaseBlockDevSave>> for BlockDevMgr {
    fn record(&self) -> Vec<BaseBlockDevSave> {
        self.block_devs.iter().map(|bd| bd.record()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{env, error::Error, path::PathBuf};

    use crate::engine::strat_engine::{
        cmd,
        keys::MemoryFilesystem,
        names::KeyDescription,
        tests::{crypt, loopbacked, real},
    };

    use super::*;

    /// Verify that initially,
    /// size() - metadata_size() = avail_space().
    /// After 2 Sectors have been allocated, that amount must also be included
    /// in balance.
    fn test_blockdevmgr_used(paths: &[&Path]) {
        let mut mgr =
            BlockDevMgr::initialize(PoolUuid::new_v4(), paths, MDADataSize::default(), None)
                .unwrap();
        assert_eq!(mgr.avail_space() + mgr.metadata_size(), mgr.size());

        let allocated = Sectors(2);
        let transaction = mgr.request_space(&[allocated]).unwrap();
        mgr.commit_space(transaction).unwrap();
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
        fn test_with_key(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let pool_uuid = PoolUuid::new_v4();
            let mut bdm = BlockDevMgr::initialize(
                pool_uuid,
                &paths[..2],
                MDADataSize::default(),
                Some(&EncryptionInfo::KeyDesc(key_desc.clone())),
            )?;

            if bdm.add(pool_uuid, &paths[2..3]).is_err() {
                Err(Box::new(StratisError::Msg(
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

    /// Test that the `BlockDevMgr` will not add devices if a different key
    /// is present in the keyring than was used to encrypted the existing
    /// devices.
    fn test_blockdevmgr_changed_key(paths: &[&Path]) {
        fn test_with_key(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let pool_uuid = PoolUuid::new_v4();
            let mut bdm = BlockDevMgr::initialize(
                pool_uuid,
                &paths[..2],
                MDADataSize::default(),
                Some(&EncryptionInfo::KeyDesc(key_desc.clone())),
            )?;

            crypt::change_key(key_desc)?;

            if bdm.add(pool_uuid, &paths[2..3]).is_ok() {
                Err(Box::new(StratisError::Msg(
                    "Adding a blockdev with a new key to an encrypted pool should fail".to_string(),
                )))
            } else {
                Ok(())
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

    fn test_clevis_initialize(paths: &[&Path]) {
        let _memfs = MemoryFilesystem::new().unwrap();
        let mut mgr = BlockDevMgr::initialize(
            PoolUuid::new_v4(),
            paths,
            MDADataSize::default(),
            Some(&EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").unwrap(), "stratis:tang:trust_url": true}),
            ))),
        )
        .unwrap();
        cmd::udev_settle().unwrap();

        matches!(
            mgr.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL").unwrap(), "stratis:tang:trust_url": true})
            ),
            Ok(false)
        );
    }

    #[test]
    fn clevis_real_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_initialize,
        );
    }

    fn test_clevis_both_initialize(paths: &[&Path]) {
        fn test_both(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let _memfs = MemoryFilesystem::new().unwrap();
            let mut mgr = BlockDevMgr::initialize(
                PoolUuid::new_v4(),
                paths,
                MDADataSize::default(),
                Some(&EncryptionInfo::Both(
                    key_desc.clone(),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
                    ),
                )),
            )?;
            cmd::udev_settle()?;

            if mgr.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
            )? {
                return Err(Box::new(StratisError::Msg(
                    "Clevis bind idempotence test failed".to_string(),
                )));
            }
            if mgr.bind_keyring(key_desc)? {
                return Err(Box::new(StratisError::Msg(
                    "Keyring bind idempotence test failed".to_string(),
                )));
            }

            if !(mgr.unbind_clevis()?) {
                return Err(Box::new(StratisError::Msg(
                    "Clevis unbind test failed".to_string(),
                )));
            }
            if mgr.unbind_clevis()? {
                return Err(Box::new(StratisError::Msg(
                    "Clevis unbind idempotence test failed".to_string(),
                )));
            }
            if mgr.unbind_keyring().is_ok() {
                return Err(Box::new(StratisError::Msg(
                    "Keyring unbind check test failed".to_string(),
                )));
            }

            if !(mgr.bind_clevis(
                "tang",
                &json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
            )?) {
                return Err(Box::new(StratisError::Msg(
                    "Clevis bind test failed".to_string(),
                )));
            }
            if !(mgr.unbind_keyring()?) {
                return Err(Box::new(StratisError::Msg(
                    "Keyring unbind test failed".to_string(),
                )));
            }
            if mgr.unbind_keyring()? {
                return Err(Box::new(StratisError::Msg(
                    "Keyring unbind idempotence test failed".to_string(),
                )));
            }
            if mgr.unbind_clevis().is_ok() {
                return Err(Box::new(StratisError::Msg(
                    "Clevis unbind check test failed".to_string(),
                )));
            }

            if !(mgr.bind_keyring(key_desc)?) {
                return Err(Box::new(StratisError::Msg(
                    "Keyring bind test failed".to_string(),
                )));
            }

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test_both);
    }

    #[test]
    fn clevis_real_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_both_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_both_initialize,
        );
    }

    fn test_clevis_both_rollback(paths: &[&Path]) {
        fn test_both(paths: &[&Path], key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
            let mut paths_vec = paths.to_vec();
            let invalid_path = PathBuf::from("/i/am/not/a/path");
            paths_vec.push(invalid_path.as_path());
            let _memfs = MemoryFilesystem::new().unwrap();
            let res = BlockDevMgr::initialize(
                PoolUuid::new_v4(),
                paths_vec.as_slice(),
                MDADataSize::default(),
                Some(&EncryptionInfo::Both(
                    key_desc.clone(),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
                    ),
                )),
            );

            if matches!(res, Ok(_)) {
                return Err(Box::new(StratisError::Msg(
                    "Initialization should fail".to_string(),
                )));
            }

            // Ensure that rollback completed successfully by trying a call that
            // should succeed.
            BlockDevMgr::initialize(
                PoolUuid::new_v4(),
                paths,
                MDADataSize::default(),
                Some(&EncryptionInfo::Both(
                    key_desc.clone(),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
                    ),
                )),
            )?;

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, test_both);
    }

    #[test]
    fn clevis_real_test_both_rollback() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_clevis_both_rollback,
        );
    }

    #[test]
    fn clevis_loop_test_both_rollback() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 4, None),
            test_clevis_both_rollback,
        );
    }
}
