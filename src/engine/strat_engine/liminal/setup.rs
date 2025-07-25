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
use itertools::Itertools;

use devicemapper::Sectors;

use crate::{
    engine::{
        strat_engine::{
            backstore::blockdev::{
                v1::{self, UnderlyingDevice},
                v2, InternalBlockDev,
            },
            crypt::handle::v1::CryptHandle,
            device::blkdev_size,
            liminal::device_info::LStratisInfo,
            serde_structs::{BackstoreSave, BaseBlockDevSave, PoolFeatures, PoolSave},
        },
        types::{BlockDevTier, DevUuid, DevicePath, Name, TokenUnlockMethod},
    },
    stratis::{StratisError, StratisResult},
};

/// Get the most recent metadata from a set of devices.
/// Returns None if no metadata found for this pool on any device. This can
/// happen if the pool was constructed but failed in the interval before the
/// metadata could be written.
/// Returns an error if there is a last update time, but no metadata could
/// be obtained from any of the devices.
///
/// Precondition: infos and bdas have identical sets of keys
pub fn get_metadata(
    infos: &HashMap<DevUuid, &LStratisInfo>,
) -> StratisResult<Option<(DateTime<Utc>, PoolSave)>> {
    // Try to read from all available devnodes that could contain most
    // recent metadata. In the event of errors, continue to try until all are
    // exhausted.
    let (_, time, info) = match infos
        .iter()
        .filter_map(|(dev_uuid, info)| {
            info.bda
                .last_update_time()
                .map(|time| (dev_uuid, *time, info))
        })
        .max_by(|(_, time1, _), (_, time2, _)| time1.cmp(time2))
    {
        Some(tup) => tup,
        None => return Ok(None),
    };

    OpenOptions::new()
        .read(true)
        .open(&info.dev_info.devnode)
        .ok()
        .and_then(|mut f| info.bda.load_state(&mut f).unwrap_or(None))
        .and_then(|data| serde_json::from_slice(&data).ok())
        .map(|psave| Some((time, psave)))
        .ok_or_else(|| {
            StratisError::Msg(
                "timestamp indicates data was written, but no data successfully read".to_string(),
            )
        })
}

/// Get the name from the most recent metadata from a set of devices.
/// Returns None if no metadata found for this pool on any device. This can
/// happen if the pool was constructed but failed in the interval before the
/// metadata could be written.
/// Returns an error if devices provided don't match the devices recorded in the
/// metadata.
pub fn get_name(infos: &HashMap<DevUuid, &LStratisInfo>) -> StratisResult<Option<Name>> {
    let found_uuids = infos.keys().copied().collect::<HashSet<_>>();
    match get_metadata(infos)? {
        Some((_, pool)) => {
            let v = [];
            let meta_uuids = pool
                .backstore
                .data_tier
                .blockdev
                .devs
                .iter()
                .map(|bd| bd.uuid)
                .chain(
                    pool.backstore
                        .cache_tier
                        .as_ref()
                        .map(|ct| ct.blockdev.devs.iter())
                        .unwrap_or_else(|| v.iter())
                        .map(|bd| bd.uuid),
                )
                .collect::<HashSet<_>>();

            if found_uuids != meta_uuids {
                return Err(StratisError::Msg(format!(
                    "UUIDs in metadata ({}) did not match UUIDs found ({})",
                    Itertools::intersperse(
                        meta_uuids.into_iter().map(|u| u.to_string()),
                        ", ".to_string(),
                    )
                    .collect::<String>(),
                    Itertools::intersperse(
                        found_uuids.into_iter().map(|u| u.to_string()),
                        ", ".to_string(),
                    )
                    .collect::<String>(),
                )));
            }

            Ok(Some(Name::new(pool.name)))
        }
        None => Ok(None),
    }
}

/// Get the feature set from the most recent metadata from a set of devices.
/// Returns None if no metadata found for this pool on any device. This can
/// happen if the pool was constructed but failed in the interval before the
/// metadata could be written.
/// Returns an error if devices provided don't match the devices recorded in the
/// metadata.
pub fn get_feature_set(
    infos: &HashMap<DevUuid, &LStratisInfo>,
) -> StratisResult<Option<Vec<PoolFeatures>>> {
    let found_uuids = infos.keys().copied().collect::<HashSet<_>>();
    match get_metadata(infos)? {
        Some((_, pool)) => {
            let v = [];
            let meta_uuids = pool
                .backstore
                .data_tier
                .blockdev
                .devs
                .iter()
                .map(|bd| bd.uuid)
                .chain(
                    pool.backstore
                        .cache_tier
                        .as_ref()
                        .map(|ct| ct.blockdev.devs.iter())
                        .unwrap_or_else(|| v.iter())
                        .map(|bd| bd.uuid),
                )
                .collect::<HashSet<_>>();

            if found_uuids != meta_uuids {
                return Err(StratisError::Msg(format!(
                    "UUIDs in metadata ({}) did not match UUIDs found ({})",
                    Itertools::intersperse(
                        meta_uuids.into_iter().map(|u| u.to_string()),
                        ", ".to_string(),
                    )
                    .collect::<String>(),
                    Itertools::intersperse(
                        found_uuids.into_iter().map(|u| u.to_string()),
                        ", ".to_string(),
                    )
                    .collect::<String>(),
                )));
            }

            Ok(Some(pool.features))
        }
        None => Ok(None),
    }
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
pub fn get_blockdevs_legacy(
    backstore_save: &BackstoreSave,
    infos: &HashMap<DevUuid, Box<LStratisInfo>>,
) -> StratisResult<(Vec<v1::StratBlockDev>, Vec<v1::StratBlockDev>)> {
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
            .or_default()
            .push((seg.start, seg.length))
    }

    if let Some(ref cache_tier) = backstore_save.cache_tier {
        for seg in cache_tier.blockdev.allocs.iter().flat_map(|i| i.iter()) {
            segment_table
                .entry(seg.parent)
                .or_default()
                .push((seg.start, seg.length))
        }
    }

    let (mut datadevs, mut cachedevs): (Vec<v1::StratBlockDev>, Vec<v1::StratBlockDev>) =
        (vec![], vec![]);
    for (_, info) in infos.iter() {
        match get_blockdev_legacy(
            info,
            &recorded_data_map,
            &recorded_cache_map,
            &segment_table,
        ) {
            Ok((tier, blockdev)) => match tier {
                BlockDevTier::Data => &mut datadevs,
                BlockDevTier::Cache => &mut cachedevs,
            }
            .push(blockdev),
            Err(e) => return Err(e),
        }
    }

    let datadevs = match check_and_sort_devs(datadevs, &recorded_data_map) {
        Ok(dd) => dd,
        Err(err) => {
            return Err(StratisError::Msg(format!(
                "Data devices did not appear consistent with metadata: {err}"
            )));
        }
    };

    let cachedevs = match check_and_sort_devs(cachedevs, &recorded_cache_map) {
        Ok(cd) => cd,
        Err(err) => {
            return Err(StratisError::Msg(format!(
                "Cache devices did not appear consistent with metadata: {err}"
            )));
        }
    };

    Ok((datadevs, cachedevs))
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
    infos: &HashMap<DevUuid, Box<LStratisInfo>>,
) -> StratisResult<(Vec<v2::StratBlockDev>, Vec<v2::StratBlockDev>)> {
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
            .or_default()
            .push((seg.start, seg.length))
    }

    if let Some(ref cache_tier) = backstore_save.cache_tier {
        for seg in cache_tier.blockdev.allocs.iter().flat_map(|i| i.iter()) {
            segment_table
                .entry(seg.parent)
                .or_default()
                .push((seg.start, seg.length))
        }
    }

    let (mut datadevs, mut cachedevs): (Vec<v2::StratBlockDev>, Vec<v2::StratBlockDev>) =
        (vec![], vec![]);
    for (_, info) in infos.iter() {
        match get_blockdev(
            info,
            &recorded_data_map,
            &recorded_cache_map,
            &segment_table,
        ) {
            Ok((tier, blockdev)) => match tier {
                BlockDevTier::Data => &mut datadevs,
                BlockDevTier::Cache => &mut cachedevs,
            }
            .push(blockdev),
            Err(e) => return Err(e),
        }
    }

    let datadevs = match check_and_sort_devs(datadevs, &recorded_data_map) {
        Ok(dd) => dd,
        Err(err) => {
            return Err(StratisError::Msg(format!(
                "Data devices did not appear consistent with metadata: {err}"
            )));
        }
    };

    let cachedevs = match check_and_sort_devs(cachedevs, &recorded_cache_map) {
        Ok(cd) => cd,
        Err(err) => {
            return Err(StratisError::Msg(format!(
                "Cache devices did not appear consistent with metadata: {err}"
            )));
        }
    };

    Ok((datadevs, cachedevs))
}

// Construct a single legacy StratBlockDev. Return the tier to which the
// blockdev has been found to belong. Returns an error if the block
// device has shrunk, no metadata can be found for the block device,
// or it is impossible to set up the device because the recorded
// allocation information is impossible.
fn get_blockdev_legacy(
    info: &LStratisInfo,
    data_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    cache_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    segment_table: &HashMap<DevUuid, Vec<(Sectors, Sectors)>>,
) -> StratisResult<(BlockDevTier, v1::StratBlockDev)> {
    let bda = info.bda.clone();

    let actual_size = OpenOptions::new()
        .read(true)
        .open(&info.dev_info.devnode)
        .map_err(StratisError::from)
        .and_then(|f| blkdev_size(&f))?;

    // Return an error if apparent size of Stratis block device appears to
    // have decreased since metadata was recorded or if size of block
    // device could not be obtained.
    let actual_size_sectors = actual_size.sectors();
    let recorded_size = bda.dev_size().sectors();
    if actual_size_sectors < recorded_size {
        let err_msg = format!(
            "Stratis device with {}, {} had recorded size {}, but actual size is less at {}",
            info.dev_info,
            bda.identifiers(),
            recorded_size,
            actual_size_sectors
        );
        return Err(StratisError::Msg(err_msg));
    }

    let dev_uuid = bda.dev_uuid();

    // Locate the device in the metadata using its uuid. Return the device
    // metadata and whether it was a cache or a datadev.
    let (tier, &(_, bd_save)) = match data_map
        .get(&dev_uuid)
        .map(|bd_save| (BlockDevTier::Data, bd_save))
        .or_else(|| {
            cache_map
                .get(&dev_uuid)
                .map(|bd_save| (BlockDevTier::Cache, bd_save))
        }) {
        Some(s) => s,
        None => {
            let err_msg = format!(
                "Stratis device with {}, {} had no record in pool metadata",
                bda.identifiers(),
                info.dev_info
            );
            return Err(StratisError::Msg(err_msg));
        }
    };

    // This should always succeed since the actual size is at
    // least the recorded size, so all segments should be
    // available to be allocated. If this fails, the most likely
    // conclusion is metadata corruption.
    let segments = segment_table.get(&dev_uuid);

    let physical_path = match &info.luks {
        Some(luks) => &luks.dev_info.devnode,
        None => &info.dev_info.devnode,
    };
    let handle = CryptHandle::setup(physical_path, TokenUnlockMethod::None, None)?;
    let underlying_device = match handle {
        Some(handle) => UnderlyingDevice::Encrypted(handle),
        None => UnderlyingDevice::Unencrypted(DevicePath::new(physical_path)?),
    };
    Ok((
        tier,
        v1::StratBlockDev::new(
            info.dev_info.device_number,
            bda,
            segments.unwrap_or(&vec![]),
            bd_save.user_info.clone(),
            bd_save.hardware_info.clone(),
            underlying_device,
        )?,
    ))
}

// Construct a single StratBlockDev. Return the tier to which the
// blockdev has been found to belong. Returns an error if the block
// device has shrunk, no metadata can be found for the block device,
// or it is impossible to set up the device because the recorded
// allocation information is impossible.
fn get_blockdev(
    info: &LStratisInfo,
    data_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    cache_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
    segment_table: &HashMap<DevUuid, Vec<(Sectors, Sectors)>>,
) -> StratisResult<(BlockDevTier, v2::StratBlockDev)> {
    let bda = info.bda.clone();
    let actual_size = OpenOptions::new()
        .read(true)
        .open(&info.dev_info.devnode)
        .map_err(StratisError::from)
        .and_then(|f| blkdev_size(&f))?;

    // Return an error if apparent size of Stratis block device appears to
    // have decreased since metadata was recorded or if size of block
    // device could not be obtained.
    let actual_size_sectors = actual_size.sectors();
    let recorded_size = bda.dev_size().sectors();
    if actual_size_sectors < recorded_size {
        let err_msg = format!(
            "Stratis device with {}, {} had recorded size {}, but actual size is less at {}",
            info.dev_info,
            bda.identifiers(),
            recorded_size,
            actual_size_sectors
        );
        return Err(StratisError::Msg(err_msg));
    }

    let dev_uuid = bda.dev_uuid();

    // Locate the device in the metadata using its uuid. Return the device
    // metadata and whether it was a cache or a datadev.
    let (tier, &(_, bd_save)) = match data_map
        .get(&dev_uuid)
        .map(|bd_save| (BlockDevTier::Data, bd_save))
        .or_else(|| {
            cache_map
                .get(&dev_uuid)
                .map(|bd_save| (BlockDevTier::Cache, bd_save))
        }) {
        Some(s) => s,
        None => {
            let err_msg = format!(
                "Stratis device with {}, {} had no record in pool metadata",
                bda.identifiers(),
                info.dev_info
            );
            return Err(StratisError::Msg(err_msg));
        }
    };

    let devnode = DevicePath::new(&info.dev_info.devnode)?;

    // This should always succeed since the actual size is at
    // least the recorded size, so all segments should be
    // available to be allocated. If this fails, the most likely
    // conclusion is metadata corruption.
    let segments = segment_table.get(&dev_uuid);
    let meta = data_map.get(&dev_uuid);
    let integrity = meta.map(|base| &base.1.integrity_meta_allocs);

    assert_eq!(info.luks, None);
    Ok((
        tier,
        v2::StratBlockDev::new(
            info.dev_info.device_number,
            bda,
            segments.unwrap_or(&vec![]),
            integrity.unwrap_or(&vec![]),
            bd_save.user_info.clone(),
            bd_save.hardware_info.clone(),
            devnode,
        )?,
    ))
}

// Verify that devices located are consistent with the metadata recorded
// and generally consistent with expectations. If all seems correct,
// sort the devices according to their order in the metadata.
fn check_and_sort_devs<B>(
    mut devs: Vec<B>,
    dev_map: &HashMap<DevUuid, (usize, &BaseBlockDevSave)>,
) -> StratisResult<Vec<B>>
where
    B: InternalBlockDev,
{
    let mut uuids = HashSet::new();
    let mut duplicate_uuids = Vec::new();
    let mut metadata_version = HashSet::new();
    for dev in &devs {
        let dev_uuid = dev.uuid();
        if !uuids.insert(dev_uuid) {
            duplicate_uuids.push(dev_uuid);
        }
        metadata_version.insert(dev.metadata_version());
    }

    if !duplicate_uuids.is_empty() {
        let err_msg = format!(
                "The following list of Stratis UUIDs were each claimed by more than one Stratis device: {}",
                duplicate_uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", ")
            );
        return Err(StratisError::Msg(err_msg));
    }

    if metadata_version.len() > 1 {
        return Err(StratisError::Msg(format!(
            "Found mismatching metadata versions across block devices: {metadata_version:?}",
        )));
    }

    let recorded_uuids: HashSet<_> = dev_map.keys().cloned().collect();
    if uuids != recorded_uuids {
        let err_msg = format!(
                "UUIDs of devices found ({}) did not correspond with UUIDs specified in the metadata for this group of devices ({})",
                uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "),
                recorded_uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(", "),
            );
        return Err(StratisError::Msg(err_msg));
    }

    // Sort the devices according to their original location in the
    // metadata. Use a faster unstable sort, because the order of
    // devs before the sort is arbitrary and does not need to be
    // preserved.
    devs.sort_unstable_by_key(|dev| dev_map[&dev.uuid()].0);
    Ok(devs)
}
