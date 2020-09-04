// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Functions to gather information necessary to set up a pool from a set
//! of unlocked devices.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
};

use chrono::{DateTime, Utc};

use devicemapper::Sectors;

use crate::{
    engine::{
        strat_engine::{
            backstore::StratBlockDev,
            device::blkdev_size,
            liminal::device_info::LStratisInfo,
            metadata::BDA,
            serde_structs::{BackstoreSave, BaseBlockDevSave, PoolSave},
        },
        types::{BlockDevPath, BlockDevTier, DevUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Given infos for each device, read and store the BDA.
///
/// Precondition: All devices represented by devnodes have been already
/// identified as having the given pool UUID and their associated device
/// UUID.
///
/// Postconditions: keys in result are equal to keys in infos OR an error
/// is returned.
pub fn get_bdas(infos: &HashMap<DevUuid, LStratisInfo>) -> StratisResult<HashMap<DevUuid, BDA>> {
    fn read_bda(info: &LStratisInfo) -> StratisResult<BDA> {
        BDA::load(&mut OpenOptions::new().read(true).open(&info.ids.devnode)?)?.ok_or_else(|| {
            StratisError::Error(format!("Failed to read BDA from device: {}", info.ids))
        })
    }

    infos
        .iter()
        .map(|(dev_uuid, info)| read_bda(info).map(|bda| (*dev_uuid, bda)))
        .collect()
}

/// Get the most recent metadata from a set of devices.
/// Returns None if no metadata found for this pool on any device. This can
/// happen if the pool was constructed but failed in the interval before the
/// metadata could be written.
/// Returns an error if there is a last update time, but no metadata could
/// be obtained from any of the devices.
///
/// Precondition: infos and bdas have identical sets of keys
pub fn get_metadata(
    infos: &HashMap<DevUuid, LStratisInfo>,
    bdas: &HashMap<DevUuid, BDA>,
) -> StratisResult<Option<(DateTime<Utc>, PoolSave)>> {
    // Most recent time should never be None if this was a properly
    // created pool; this allows for the method to be called in other
    // circumstances.
    let most_recent_time = {
        match bdas
            .iter()
            .filter_map(|(_, bda)| bda.last_update_time())
            .max()
        {
            Some(time) => time,
            None => return Ok(None),
        }
    };

    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    bdas.iter()
        .filter_map(|(uuid, bda)| {
            if bda.last_update_time() == Some(most_recent_time) {
                OpenOptions::new()
                    .read(true)
                    .open(
                        &infos
                            .get(uuid)
                            .expect("equal sets of UUID keys")
                            .ids
                            .devnode,
                    )
                    .ok()
                    .and_then(|mut f| bda.load_state(&mut f).unwrap_or(None))
                    .and_then(|data| serde_json::from_slice(&data).ok())
            } else {
                None
            }
        })
        .next()
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                "timestamp indicates data was written, but no data successfully read".into(),
            )
        })
        .map(|psave| Some((*most_recent_time, psave)))
}

/// Get all the blockdevs corresponding to this pool that can be obtained from
/// the given devices. Sort the blockdevs in the order in which they were
/// recorded in the metadata.
/// Returns an error if the blockdevs obtained do not match the metadata.
/// Returns a tuple, of which the first are the data devs, and the second
/// are the devs that support the cache tier.
/// Precondition: Every device in infos has already been determined to
/// belong to one pool; all BDAs agree on their pool UUID, set of keys in
/// infos and bdas are identical.
pub fn get_blockdevs(
    backstore_save: &BackstoreSave,
    infos: &HashMap<DevUuid, LStratisInfo>,
    mut bdas: HashMap<DevUuid, BDA>,
) -> StratisResult<(Vec<StratBlockDev>, Vec<StratBlockDev>)> {
    let recorded_data_map: HashMap<DevUuid, (usize, &BaseBlockDevSave)> = backstore_save
        .data_tier
        .blockdev
        .devs
        .iter()
        .enumerate()
        .map(|(i, bds)| (bds.uuid, (i, bds)))
        .collect();

    let recorded_cache_map: HashMap<DevUuid, (usize, &BaseBlockDevSave)> =
        match backstore_save.cache_tier {
            Some(ref cache_tier) => cache_tier
                .blockdev
                .devs
                .iter()
                .enumerate()
                .map(|(i, bds)| (bds.uuid, (i, bds)))
                .collect(),
            None => HashMap::new(),
        };

    let mut segment_table: HashMap<DevUuid, Vec<(Sectors, Sectors)>> = HashMap::new();
    for seg in &backstore_save.data_tier.blockdev.allocs[0] {
        segment_table
            .entry(seg.parent)
            .or_insert_with(Vec::default)
            .push((seg.start, seg.length))
    }

    if let Some(ref cache_tier) = backstore_save.cache_tier {
        for seg in cache_tier.blockdev.allocs.iter().flat_map(|i| i.iter()) {
            segment_table
                .entry(seg.parent)
                .or_insert_with(Vec::default)
                .push((seg.start, seg.length))
        }
    }

    // Construct a single StratBlockDev. Return the tier to which the
    // blockdev has been found to belong. Returns an error if the block
    // device has shrunk, no metadata can be found for the block device,
    // or it is impossible to set up the device because the recorded
    // allocation information is impossible.
    fn get_blockdev(
        info: &LStratisInfo,
        bda: BDA,
        data_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
        cache_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
        segment_table: &HashMap<DevUuid, Vec<(Sectors, Sectors)>>,
    ) -> StratisResult<(BlockDevTier, StratBlockDev)> {
        // Return an error if apparent size of Stratis block device appears to
        // have decreased since metadata was recorded or if size of block
        // device could not be obtained.
        blkdev_size(&OpenOptions::new().read(true).open(&info.ids.devnode)?).and_then(
            |actual_size| {
                let actual_size_sectors = actual_size.sectors();
                let recorded_size = bda.dev_size().sectors();
                if actual_size_sectors < recorded_size {
                    let err_msg = format!(
                    "Stratis device with {} had recorded size {}, but actual size is less at {}",
                    info.ids,
                    recorded_size,
                    actual_size_sectors
                );
                    Err(StratisError::Engine(ErrorEnum::Error, err_msg))
                } else {
                    Ok(())
                }
            },
        )?;

        let dev_uuid = bda.dev_uuid();

        // Locate the device in the metadata using its uuid. Return the device
        // metadata and whether it was a cache or a datadev.
        let (tier, &(_, bd_save)) = data_map
            .get(&dev_uuid)
            .map(|bd_save| (BlockDevTier::Data, bd_save))
            .or_else(|| {
                cache_map
                    .get(&dev_uuid)
                    .map(|bd_save| (BlockDevTier::Cache, bd_save))
            })
            .ok_or_else(|| {
                let err_msg = format!(
                    "Stratis device with {} had no record in pool metadata",
                    info.ids
                );
                StratisError::Engine(ErrorEnum::NotFound, err_msg)
            })?;

        // This should always succeed since the actual size is at
        // least the recorded size, so all segments should be
        // available to be allocated. If this fails, the most likely
        // conclusion is metadata corruption.
        let segments = segment_table.get(&dev_uuid);

        let (path, key_description) = match &info.luks {
            Some(luks) => (
                BlockDevPath::mapped_device_path(&luks.ids.devnode, &info.ids.devnode)?,
                Some(&luks.key_description),
            ),
            None => (BlockDevPath::physical_device_path(&info.ids.devnode), None),
        };

        Ok((
            tier,
            StratBlockDev::new(
                info.ids.device_number,
                path,
                bda,
                segments.unwrap_or(&vec![]),
                bd_save.user_info.clone(),
                bd_save.hardware_info.clone(),
                key_description,
            )?,
        ))
    }

    let (mut datadevs, mut cachedevs): (Vec<StratBlockDev>, Vec<StratBlockDev>) = (vec![], vec![]);
    for (dev_uuid, info) in infos {
        get_blockdev(
            info,
            bdas.remove(dev_uuid)
                .expect("sets of keys in bdas and infos are identical"),
            &recorded_data_map,
            &recorded_cache_map,
            &segment_table,
        )
        .map(|(tier, blockdev)| {
            match tier {
                BlockDevTier::Data => &mut datadevs,
                BlockDevTier::Cache => &mut cachedevs,
            }
            .push(blockdev)
        })?;
    }

    // Verify that devices located are consistent with the metadata recorded
    // and generally consistent with expectations. If all seems correct,
    // sort the devices according to their order in the metadata.
    fn check_and_sort_devs(
        mut devs: Vec<StratBlockDev>,
        dev_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    ) -> StratisResult<Vec<StratBlockDev>> {
        let mut uuids = HashSet::new();
        let mut duplicate_uuids = Vec::new();
        for dev in &devs {
            let dev_uuid = dev.uuid();
            if !uuids.insert(dev_uuid) {
                duplicate_uuids.push(dev_uuid);
            }
        }

        if !duplicate_uuids.is_empty() {
            let err_msg = format!(
                "The following list of Stratis UUIDs were each claimed by more than one Stratis device: {}",
                duplicate_uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", ")
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        let recorded_uuids: HashSet<_> = dev_map.keys().cloned().collect();
        if uuids != recorded_uuids {
            let err_msg = format!(
                "UUIDs of devices found ({}) did not correspond with UUIDs specified in the metadata for this group of devices ({})",
                uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", "),
                recorded_uuids.iter().map(|u| u.to_simple_ref().to_string()).collect::<Vec<_>>().join(", "),
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, err_msg));
        }

        // Sort the devices according to their original location in the
        // metadata. Use a faster unstable sort, because the order of
        // devs before the sort is arbitrary and does not need to be
        // preserved.
        devs.sort_unstable_by_key(|dev| dev_map[&dev.uuid()].0);
        Ok(devs)
    }

    let datadevs = check_and_sort_devs(datadevs, &recorded_data_map).map_err(|err| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "Data devices did not appear consistent with metadata: {}",
                err
            ),
        )
    })?;

    let cachedevs = check_and_sort_devs(cachedevs, &recorded_cache_map).map_err(|err| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "Cache devices did not appear consistent with metadata: {}",
                err
            ),
        )
    })?;

    Ok((datadevs, cachedevs))
}

#[cfg(test)]
mod tests {
    use std::{convert::TryFrom, error::Error, path::Path};

    use uuid::Uuid;

    use devicemapper::Device;

    use crate::engine::{
        strat_engine::{
            backstore::{Backstore, LuksInfo, StratisInfo},
            liminal::device_info::{LInfo, LLuksInfo},
            metadata::{MDADataSize, StratisIdentifiers},
            serde_structs::Recordable,
            tests::{crypt, loopbacked, real},
        },
        types::{KeyDescription, PoolUuid},
    };

    use super::*;

    const CACHE_BLOCK_SIZE: Sectors = Sectors(2048); // 1024 KiB
    const INITIAL_BACKSTORE_ALLOCATION: Sectors = CACHE_BLOCK_SIZE;

    // Generate data that might be associated with this backstore while
    // bringing up a pool.
    fn blockdev_data(
        backstore: &Backstore,
        pool_uuid: PoolUuid,
    ) -> Result<HashMap<DevUuid, LStratisInfo>, nix::Error> {
        fn stratis_info(
            pool_uuid: PoolUuid,
            device_uuid: DevUuid,
            device_number: Device,
            devnode: &Path,
        ) -> StratisInfo {
            StratisInfo {
                identifiers: StratisIdentifiers {
                    pool_uuid,
                    device_uuid,
                },
                device_number,
                devnode: devnode.to_owned(),
            }
        }

        let encrypted = backstore.data_tier_is_encrypted();
        let key_description = backstore.data_key_desc();
        backstore
            .blockdevs()
            .iter()
            .map(|(device_uuid, tier, blockdev)| {
                if encrypted && *tier == BlockDevTier::Data {
                    let luks_path = blockdev.devnode().physical_path();
                    let luks_device_number =
                        nix::sys::stat::stat(luks_path).map(|res| Device::from(res.st_rdev))?;
                    let luks_info: LLuksInfo = LuksInfo {
                        info: stratis_info(pool_uuid, *device_uuid, luks_device_number, luks_path),
                        key_description: KeyDescription::try_from(
                            key_description
                                .as_ref()
                                .expect("must exist, because encrypted")
                                .as_application_str()
                                .to_string(),
                        )
                        .expect("round trip"),
                    }
                    .into();
                    if let LInfo::Stratis(info) = LInfo::update(
                        LInfo::Luks(luks_info),
                        LInfo::Stratis(
                            stratis_info(
                                pool_uuid,
                                *device_uuid,
                                *blockdev.device(),
                                blockdev.devnode().metadata_path(),
                            )
                            .into(),
                        ),
                    )
                    .expect("the two elements are compatible")
                    {
                        Ok(info)
                    } else {
                        unreachable!("if one of the elements is a Stratis info, the result must be")
                    }
                } else {
                    Ok(stratis_info(
                        pool_uuid,
                        *device_uuid,
                        *blockdev.device(),
                        blockdev.devnode().metadata_path(),
                    )
                    .into())
                }
                .map(|info| (*device_uuid, info))
            })
            .collect()
    }

    // Verify that get_bdas(), get_metadata(), get_blockdevs() discover the
    // correct information to build a backstore object with the same metadata
    // as when initialized and a cache added.
    fn test_setup(
        paths: &[&Path],
        key_description: Option<&KeyDescription>,
    ) -> Result<(), Box<dyn Error>> {
        if paths.len() < 2 {
            return Err(Box::new(StratisError::Error(
                "'paths' values does not have the number of elements required by the test"
                    .to_string(),
            )));
        }

        let (paths1, paths2) = paths.split_at(paths.len() / 2);

        let pool_uuid = Uuid::new_v4();

        let (backstore_save, infos): (_, HashMap<DevUuid, LStratisInfo>) = {
            let mut backstore =
                Backstore::initialize(pool_uuid, paths1, MDADataSize::default(), key_description)?;

            // Allocate space from the backstore so that the cap device is made.
            backstore.alloc(pool_uuid, &[INITIAL_BACKSTORE_ALLOCATION])?;

            let old_device = backstore.device();

            backstore.init_cache(pool_uuid, paths2)?;

            if backstore.device() == old_device {
                return Err(Box::new(StratisError::Error(
                    "Backstore device is the same as the device before the cache was initialized"
                        .to_string(),
                )));
            }

            (backstore.record(), blockdev_data(&backstore, pool_uuid)?)
        };

        {
            let bdas = get_bdas(&infos)?;
            let (datadevs, cachedevs) = get_blockdevs(&backstore_save, &infos, bdas)?;
            let mut backstore = Backstore::setup(
                pool_uuid,
                &backstore_save,
                datadevs,
                cachedevs,
                Utc::now(),
                key_description,
            )?;

            let backstore_save2 = backstore.record();
            assert_eq!(backstore_save.cache_tier, backstore_save2.cache_tier);
            assert_eq!(backstore_save.data_tier, backstore_save2.data_tier);

            backstore.teardown()?;
        }

        {
            let bdas = get_bdas(&infos).unwrap();
            let (datadevs, cachedevs) = get_blockdevs(&backstore_save, &infos, bdas)?;
            let mut backstore = Backstore::setup(
                pool_uuid,
                &backstore_save,
                datadevs,
                cachedevs,
                Utc::now(),
                key_description,
            )?;

            let backstore_save2 = backstore.record();
            assert_eq!(backstore_save.cache_tier, backstore_save2.cache_tier);
            assert_eq!(backstore_save.data_tier, backstore_save2.data_tier);

            backstore.destroy()?;
        }
        Ok(())
    }

    fn test_setup_no_crypt(paths: &[&Path]) {
        test_setup(paths, None).unwrap()
    }

    fn test_setup_crypt(paths: &[&Path]) {
        fn call_crypt_test(
            paths: &[&Path],
            key_description: &KeyDescription,
            _: Option<()>,
        ) -> Result<(), Box<dyn Error>> {
            test_setup(paths, Some(key_description))
        }

        crypt::insert_and_cleanup_key(paths, call_crypt_test)
    }

    #[test]
    fn loop_test_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_setup_no_crypt,
        );
    }

    #[test]
    fn real_test_setup() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_setup_no_crypt,
        );
    }

    #[test]
    fn travis_test_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_setup_no_crypt,
        );
    }

    #[test]
    fn loop_test_crypt_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_setup_crypt,
        );
    }

    #[test]
    fn real_test_crypt_setup() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_setup_crypt,
        );
    }

    #[test]
    fn travis_test_crypt_setup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_setup_crypt,
        );
    }
}
