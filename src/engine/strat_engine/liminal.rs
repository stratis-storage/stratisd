// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::OpenOptions,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::Sectors;

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                identify_block_device, DeviceInfo, LuksInfo, StratBlockDev, StratisIdentifiers,
                StratisInfo, BDA,
            },
            device::blkdev_size,
            devlinks::setup_pool_devlinks,
            pool::StratPool,
            serde_structs::{BackstoreSave, BaseBlockDevSave, PoolSave},
        },
        structures::Table,
        types::{BlockDevPath, BlockDevTier, DevUuid, KeyDescription, Name, PoolUuid},
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

    // Verify that devices located are congruent with the metadata recorded
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

    let datadevs = check_and_sort_devs(datadevs, &recorded_data_map)?;
    let cachedevs = check_and_sort_devs(cachedevs, &recorded_cache_map)?;

    Ok((datadevs, cachedevs))
}

/// Info for a discovered Luks Device belonging to Stratis.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LLuksInfo {
    /// Generic information + Stratis identifiers
    pub ids: StratisInfo,
    pub key_description: KeyDescription,
}

impl fmt::Display for LLuksInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}, key description: {}",
            self.ids,
            self.key_description.as_application_str()
        )
    }
}

impl From<LuksInfo> for LLuksInfo {
    fn from(info: LuksInfo) -> LLuksInfo {
        LLuksInfo {
            ids: info.info,
            key_description: info.key_description,
        }
    }
}

impl<'a> Into<Value> for &'a LLuksInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = json!({
            "key_description": Value::from(self.key_description.as_application_str())
        });
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) = <&StratisInfo as Into<Value>>::into(&self.ids) {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object");
        };
        json
    }
}

/// Info for a Stratis device.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LStratisInfo {
    /// Generic information + Stratis identifiers
    pub ids: StratisInfo,
    /// Luks information. The information will be set if this is a Stratis
    /// device which is an activated encrypted device, and will be
    /// information about that physical device.
    pub luks: Option<LLuksInfo>,
}

impl fmt::Display for LStratisInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(info) = &self.luks {
            write!(
                f,
                "logical device with {} and physical device with {}",
                self.ids, info
            )
        } else {
            write!(f, "{}", self.ids)
        }
    }
}

impl From<StratisInfo> for LStratisInfo {
    fn from(info: StratisInfo) -> LStratisInfo {
        LStratisInfo {
            ids: info,
            luks: None,
        }
    }
}

impl<'a> Into<Value> for &'a LStratisInfo {
    // Precondition: (&StratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        let mut json = self
            .luks
            .as_ref()
            .map(|luks| json!({ "luks": <&LLuksInfo as Into<Value>>::into(luks) }))
            .unwrap_or_else(|| json!({}));
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) = <&StratisInfo as Into<Value>>::into(&self.ids) {
                    map.into_iter()
                } else {
                    unreachable!("StratisInfo conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object");
        };
        json
    }
}

impl LStratisInfo {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(match &self.luks {
            None => true,
            Some(luks) =>
                luks.ids.identifiers == self.ids.identifiers
                    && luks.ids.devnode != self.ids.devnode
                    && luks.ids.device_number != self.ids.device_number,
        });
    }
}

/// A unifying Info struct for Stratis or Luks devices
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum LInfo {
    /// A Stratis device, which may be an encrypted device
    Stratis(LStratisInfo),
    /// A LUKS device
    Luks(LLuksInfo),
}

impl fmt::Display for LInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LInfo::Stratis(info) => write!(f, "Stratis device with {}", info),
            LInfo::Luks(info) => write!(f, "LUKS device belonging to Stratis with {}", info),
        }
    }
}

impl From<DeviceInfo> for LInfo {
    fn from(info: DeviceInfo) -> LInfo {
        match info {
            DeviceInfo::Luks(info) => LInfo::Luks(info.into()),
            DeviceInfo::Stratis(info) => LInfo::Stratis(info.into()),
        }
    }
}

impl<'a> Into<Value> for &'a LInfo {
    // Precondition: (&LStratisInfo).into() pattern matches Value::Object()
    // Precondition: (&LLuksInfo).into() pattern matches Value::Object()
    fn into(self) -> Value {
        match self {
            LInfo::Stratis(info) => info.into(),
            LInfo::Luks(info) => info.into(),
        }
    }
}

impl LInfo {
    fn stratis_identifiers(&self) -> StratisIdentifiers {
        match self {
            LInfo::Luks(info) => info.ids.identifiers,
            LInfo::Stratis(info) => info.ids.identifiers,
        }
    }
}

/// On an error, whether this set of devices is hopeless or just errored
#[derive(Debug)]
enum Destination {
    Hopeless(String),
    Errored(String),
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Destination::Hopeless(val) => write!(f, "{}", val),
            Destination::Errored(val) => write!(f, "{}", val),
        }
    }
}

/// Devices which stratisd has discovered but which have not been assembled
/// into pools.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct LiminalDevices {
    /// Sets of devices which have not been promoted to pools, but which
    /// may still have a chance.
    errored_pool_devices: HashMap<PoolUuid, HashMap<DevUuid, LInfo>>,
    /// Sets of devices which possess some internal contradiction which makes
    /// it impossible for them to be made into sensible pools ever.
    /// Use a HashSet to store the infos for each pool, as the problem that
    /// makes the set hopeless may be duplicate device UUIDs, so that a
    /// map with device UUID keys would not be able to manage all the devices.
    hopeless_device_sets: HashMap<PoolUuid, HashSet<LInfo>>,
}

impl LiminalDevices {
    #[allow(dead_code)]
    fn invariant(&self) {
        assert!(self
            .errored_pool_devices
            .keys()
            .cloned()
            .collect::<HashSet<PoolUuid>>()
            .intersection(
                &self
                    .hopeless_device_sets
                    .keys()
                    .cloned()
                    .collect::<HashSet<PoolUuid>>()
            )
            .next()
            .is_none());
    }

    /// Take maps of pool UUIDs to sets of devices and return a list of
    /// information about created pools.
    ///
    /// Precondition: No pools have yet been set up, i.e., it is unnecessary
    /// to check for membership in any of the existing categories of device
    /// sets.
    pub fn setup_pools(
        &mut self,
        all_devices: (
            HashMap<PoolUuid, Vec<LuksInfo>>,
            HashMap<PoolUuid, Vec<StratisInfo>>,
        ),
    ) -> Vec<(Name, PoolUuid, StratPool)> {
        let table = Table::default();
        let (mut luks_devices, mut stratis_devices) = all_devices;

        let pool_uuids: HashSet<PoolUuid> = luks_devices
            .keys()
            .cloned()
            .collect::<HashSet<PoolUuid>>()
            .union(&stratis_devices.keys().cloned().collect())
            .cloned()
            .collect();

        pool_uuids
            .iter()
            .filter_map(|pool_uuid| {
                let luks_infos = luks_devices.remove(pool_uuid);
                let stratis_infos = stratis_devices.remove(pool_uuid);
                let mut infos: Vec<LInfo> = stratis_infos
                    .unwrap_or_else(Vec::new)
                    .drain(..)
                    .map(|info| LInfo::Stratis(info.into()))
                    .chain(
                        luks_infos
                            .unwrap_or_else(Vec::new)
                            .drain(..)
                            .map(|info| LInfo::Luks(info.into())),
                    )
                    .collect();

                let mut info_map = Some(HashMap::new());
                while !infos.is_empty() && info_map.is_some() {
                    let info: LInfo = infos.pop().expect("!infos.is_empty()");
                    info_map = match self.process_info_add(info_map.expect("loop condition"), info)
                    {
                        Err(mut hopeless) => {
                            hopeless.extend(infos.drain(..));
                            self.hopeless_device_sets.insert(*pool_uuid, hopeless);
                            None
                        }
                        Ok(info_map) => Some(info_map),
                    }
                }

                match info_map {
                    Some(info_map) => self
                        .try_setup_pool(&table, *pool_uuid, info_map)
                        .map(|(pool_name, pool)| (pool_name, *pool_uuid, pool)),
                    None => None,
                }
            })
            .collect::<Vec<(Name, PoolUuid, StratPool)>>()
    }

    /// Given a set of devices, try to set up a pool.
    /// Return the pool information if a pool is set up. Otherwise, distribute
    /// the pool information to the appropriate data structure.
    /// Do not attempt setup if the pool contains any unopened devices.
    ///
    /// If there is a name conflict between the set of devices in devices
    /// and some existing pool, return an error.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.errored_pool_devices.get(pool_uuid).is_none() &&
    ///               self.hopeless_device_sets.get(pool_uuid).is_none()
    fn try_setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        mut infos: HashMap<DevUuid, LInfo>,
    ) -> Option<(Name, StratPool)> {
        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.errored_pool_devices.get(&pool_uuid).is_none());
        assert!(self.hopeless_device_sets.get(&pool_uuid).is_none());

        // Setup a pool from constituent devices in the context of some already
        // setup pools.
        // Return None if the pool's metadata was not found. This is a
        // legitimate non-error condition, which may result if only a subset
        // of the pool's devices are in the set of devices being used.
        // Return an error on all other errors. Note that any one of these
        // errors could represent a temporary condition, that could be changed
        // by finding another device. So it is reasonable to treat them all
        // as loggable at the warning level, but not at the error level.
        // Precondition: every device in devices has already been determined to belong
        // to the pool with pool_uuid.
        fn setup_pool(
            pools: &Table<StratPool>,
            pool_uuid: PoolUuid,
            infos: &HashMap<DevUuid, LStratisInfo>,
        ) -> Result<Option<(Name, StratPool)>, Destination> {
            let bdas = match get_bdas(infos) {
                Err(err) => Err(
                    Destination::Errored(format!(
                        "There was an error encountered when reading the BDAs for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err))),
                Ok(infos) => Ok(infos),
            }?;

            if let Some((dev_uuid, bda)) = bdas.iter().find(|(dev_uuid, bda)| {
                **dev_uuid != bda.dev_uuid() || pool_uuid != bda.pool_uuid()
            }) {
                return Err(
                    Destination::Hopeless(format!(
                        "Mismatch between Stratis identifiers previously read and those found on some BDA: {} != {}",
                        StratisIdentifiers::new(pool_uuid, *dev_uuid),
                        StratisIdentifiers::new(bda.pool_uuid(), bda.dev_uuid())
                        )));
            }

            let (timestamp, metadata) = match get_metadata(infos, &bdas) {
                Err(err) => return Err(
                    Destination::Errored(format!(
                        "There was an error encountered when reading the metadata for the devices found for pool with UUID {}: {}",
                        pool_uuid.to_simple_ref(),
                        err))),
                Ok(None) => return Ok(None),
                Ok(Some((timestamp, metadata))) => (timestamp, metadata),
            };

            if let Some((uuid, _)) = pools.get_by_name(&metadata.name) {
                return Err(
                    Destination::Errored(format!(
                        "There is a pool name conflict. The devices currently being processed have been identified as belonging to the pool with UUID {} and name {}, but a pool with the same name and UUID {} is already active",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        uuid.to_simple_ref())));
            }

            let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, infos, bdas) {
                Err(err) => return Err(
                    Destination::Errored(format!(
                        "There was an error encountered when calculating the block devices for pool with UUID {} and name {}: {}",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        err))),
                Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
            };

            let num_with_luks = datadevs
                .iter()
                .filter_map(|sbd| sbd.key_description())
                .count();

            if num_with_luks != 0 && num_with_luks != datadevs.len() {
                // NOTE: This is not actually a hopeless situation. It may be
                // that a LUKS device owned by Stratis corresponding to a
                // Stratis device has just not been discovered yet. If it
                // is, the appropriate info will be updated, and setup may
                // yet succeed.
                return Err(
                    Destination::Errored(format!(
                            "Some data devices in the set belonging to pool with UUID {} and name {} appear to be encrypted devices managed by Stratis, and some do not",
                            pool_uuid.to_simple_ref(),
                            &metadata.name)));
            }

            if num_with_luks != 0 {
                let num_key_descriptions = datadevs
                    .iter()
                    .map(|sbd| {
                        sbd.key_description()
                            .expect("num_with_luks != 0 -> num_with_luks == datadevs.len()")
                    })
                    .collect::<HashSet<&KeyDescription>>()
                    .iter()
                    .count();
                if num_key_descriptions != 1 {
                    return Err(
                        Destination::Hopeless(format!(
                            "Data devices in the set belonging to pool with UUID {} and name {} do not agree on their key description",
                            pool_uuid.to_simple_ref(),
                            &metadata.name)));
                }
            }

            StratPool::setup(pool_uuid, datadevs, cachedevs, timestamp, &metadata, None)
                .map_err(|err| {
                    Destination::Errored(
                    format!(
                        "An attempt to set up pool with UUID {} from the assembled devices failed: {}",
                        pool_uuid.to_simple_ref(),
                        err
                    ))
                })
                .map(Some)
        }

        if infos.iter().any(|(_, info)| match info {
            LInfo::Luks(_) => true,
            LInfo::Stratis(_) => false,
        }) {
            self.errored_pool_devices.insert(pool_uuid, infos);
            return None;
        }

        let mut infos = infos
            .drain()
            .map(|(pool_uuid, info)| match info {
                LInfo::Luks(_) => unreachable!("otherwise, returned in line above"),
                LInfo::Stratis(info) => (pool_uuid, info),
            })
            .collect();

        let result = setup_pool(pools, pool_uuid, &infos);

        if let Err(err) = &result {
            warn!("{}", err);
        }

        match result {
            Ok(Some((pool_name, pool))) => {
                setup_pool_devlinks(&pool_name, &pool);
                info!(
                    "Pool with name \"{}\" and UUID \"{}\" set up",
                    pool_name,
                    pool_uuid.to_simple_ref()
                );
                Some((pool_name, pool))
            }
            Err(Destination::Hopeless(_)) => {
                self.hopeless_device_sets.insert(
                    pool_uuid,
                    infos
                        .drain()
                        .map(|(_, info)| LInfo::Stratis(info))
                        .collect(),
                );
                None
            }
            Err(Destination::Errored(_)) | Ok(None) => {
                self.errored_pool_devices.insert(
                    pool_uuid,
                    infos
                        .drain()
                        .map(|(pool_uuid, info)| (pool_uuid, LInfo::Stratis(info)))
                        .collect(),
                );
                None
            }
        }
    }

    /// Process a device for inclusion in a set of devices.
    /// If processing was succesful, return the original set, modified.
    /// If unsuccesful, return the new set to be added to the hopeless
    /// category.
    fn process_info_add(
        &mut self,
        mut devices: HashMap<DevUuid, LInfo>,
        info: LInfo,
    ) -> Result<HashMap<DevUuid, LInfo>, HashSet<LInfo>> {
        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn luks_luks_compatible(info_1: &LLuksInfo, info_2: &LLuksInfo) -> bool {
            assert_eq!(info_1.ids.identifiers, info_2.ids.identifiers);
            info_1.ids.device_number == info_2.ids.device_number
                && info_1.key_description == info_2.key_description
        }

        // Returns true if the information found via udev for two devices is
        // compatible, otherwise false.
        // Precondition: Stratis identifiers of devices are the same
        fn stratis_stratis_compatible(info_1: &LStratisInfo, info_2: &LStratisInfo) -> bool {
            assert_eq!(info_1.ids.identifiers, info_2.ids.identifiers);
            info_1.ids.device_number == info_2.ids.device_number
                && match (info_1.luks.as_ref(), info_2.luks.as_ref()) {
                    (Some(luks_1), Some(luks_2)) => luks_luks_compatible(luks_1, luks_2),
                    _ => true,
                }
        }

        // Combine two devices which have identical pool and device UUIDs.
        // The first argument is the older information, the second the newer.
        // Allow the newer information to supplant the older.
        // Precondition: the newer information must always represent a single
        // device, so the luks field of a newly discovered Stratis device
        // must always be None.
        fn combine_two_devices(
            info_1: LInfo,
            info_2: LInfo,
        ) -> Result<LInfo, (String, LInfo, LInfo)> {
            match (info_1, info_2) {
                (LInfo::Luks(luks_info), LInfo::Stratis(strat_info)) => {
                    assert_eq!(strat_info.luks, None);
                    Ok(LInfo::Stratis(LStratisInfo {
                        ids: strat_info.ids,
                        luks: Some(luks_info),
                    }))
                }
                (LInfo::Stratis(strat_info), LInfo::Luks(luks_info)) => {
                    if let Some(luks) = strat_info.luks.as_ref() {
                        if !luks_luks_compatible(luks, &luks_info) {
                            let (info_1, info_2) =
                                (LInfo::Stratis(strat_info), LInfo::Luks(luks_info));
                            let err_msg = format!(
                                "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                                info_1,
                                info_2);

                            return Err((err_msg, info_1, info_2));
                        }
                    }
                    Ok(LInfo::Stratis(LStratisInfo {
                        ids: strat_info.ids,
                        luks: Some(luks_info),
                    }))
                }
                (LInfo::Luks(luks_info_1), LInfo::Luks(luks_info_2)) => {
                    if !luks_luks_compatible(&luks_info_1, &luks_info_2) {
                        let (info_1, info_2) = (LInfo::Luks(luks_info_1), LInfo::Luks(luks_info_2));
                        let err_msg = format!(
                            "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                            info_1,
                            info_2);
                        Err((err_msg, info_1, info_2))
                    } else {
                        Ok(LInfo::Luks(luks_info_2))
                    }
                }
                (LInfo::Stratis(strat_info_1), LInfo::Stratis(strat_info_2)) => {
                    if !stratis_stratis_compatible(&strat_info_1, &strat_info_2) {
                        let (info_1, info_2) =
                            (LInfo::Stratis(strat_info_1), LInfo::Stratis(strat_info_2));
                        let err_msg = format!(
                            "Information about previously discovered device {} incompatible with information about newly discovered device {}",
                            info_1,
                            info_2);
                        Err((err_msg, info_1, info_2))
                    } else {
                        Ok(LInfo::Stratis(LStratisInfo {
                            ids: strat_info_2.ids,
                            luks: strat_info_2.luks.or(strat_info_1.luks),
                        }))
                    }
                }
            }
        }

        let stratis_identifiers = info.stratis_identifiers();
        let device_uuid = stratis_identifiers.device_uuid;

        match devices.remove(&device_uuid) {
            None => {
                info!(
                    "device with Stratis identifiers {} discovered, i.e., identified for the first time during this execution of stratisd",
                    stratis_identifiers);
                devices.insert(device_uuid, info);
                Ok(devices)
            }
            Some(removed) => match combine_two_devices(removed, info) {
                Err((err, removed, info)) => {
                    warn!(
                        "Moving set of devices with pool UUID {} to hopeless sets because {}",
                        stratis_identifiers.pool_uuid, err
                    );
                    let mut hopeless: HashSet<LInfo> =
                        devices.drain().map(|(_, info)| info).collect();
                    hopeless.insert(removed);
                    hopeless.insert(info);
                    Err(hopeless)
                }
                Ok(info) => {
                    devices.insert(device_uuid, info);
                    Ok(devices)
                }
            },
        }
    }

    /// Process a device for removal from a set of devices.
    fn process_info_remove(&mut self, devices: &mut HashMap<DevUuid, LInfo>, info: LInfo) {
        // Combine two devices which have identical pool and device UUIDs.
        // The first argument is the existing information, the second is the
        // information about the removed device, where "removed" means there
        // was a udev "remove" event and this info has been found out about the
        // device attached to the event.
        fn combine_remove_devices(info_1: LInfo, info_2: LInfo) -> Option<LInfo> {
            match (info_1, info_2) {
                (LInfo::Luks(luks_info), LInfo::Stratis(_)) => Some(LInfo::Luks(luks_info)),
                (LInfo::Stratis(strat_info), LInfo::Luks(luks_info)) => {
                    if let Some(luks) = &strat_info.luks {
                        if luks.ids.device_number != luks_info.ids.device_number {
                            warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining logical device with {} among the set of devices known to belong to pool with UUID {}",
                                luks_info,
                                strat_info,
                                strat_info.ids.identifiers.pool_uuid);
                        } else {
                            warn!("Received udev remove event on a device with {} that appeared to belong to Stratis, but the logical device information is still present; retaining the logical device with the original encryption information",
                                  luks_info);
                        }
                    }
                    Some(LInfo::Stratis(strat_info))
                }
                (LInfo::Stratis(info_1), LInfo::Stratis(info_2)) => {
                    if info_1.ids.device_number != info_2.ids.device_number {
                        warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining duplicate device {} among the set of devices known to belong to pool with UUID {}",
                              info_2,
                              info_1,
                              info_1.ids.identifiers.pool_uuid);
                        Some(LInfo::Stratis(info_1))
                    } else {
                        info_1.luks.map(LInfo::Luks)
                    }
                }
                (LInfo::Luks(info_1), LInfo::Luks(info_2)) => {
                    if info_1.ids.device_number != info_2.ids.device_number {
                        warn!("Received udev remove event on a device with {} that stratisd does not know about; retaining duplicate device {} among the set of devices known to belong to pool with UUID {}",
                              info_2,
                              info_1,
                              info_1.ids.identifiers.pool_uuid);
                        Some(LInfo::Luks(info_1))
                    } else {
                        None
                    }
                }
            }
        }

        let stratis_identifiers = info.stratis_identifiers();
        let device_uuid = stratis_identifiers.device_uuid;

        if let Some(removed) = devices.remove(&device_uuid) {
            if let Some(info) = combine_remove_devices(removed, info) {
                devices.insert(device_uuid, info);
            }
        }
    }

    /// Given some information gathered about a single Stratis device, determine
    /// whether or not a pool can be constructed, and if it can, construct the
    /// pool and return the newly constructed pool. If the device appears to
    /// belong to a pool that has already been set up assume that no further
    /// processing is required and return None. If there is an error
    /// constructing the pool, retain the set of devices.
    pub fn block_evaluate(
        &mut self,
        pools: &Table<StratPool>,
        event: &libudev::Event,
    ) -> Option<(PoolUuid, Name, StratPool)> {
        let event_type = event.event_type();
        if event_type == libudev::EventType::Add || event_type == libudev::EventType::Change {
            identify_block_device(event.device()).and_then(move |info| {
                let info: LInfo = info.into();
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    if pool.get_strat_blockdev(device_uuid).is_none() {
                        warn!("Found a device with {} that identifies itself as belonging to pool with UUID {}, but that pool is already up and running and does not appear to contain the device",
                              info,
                              pool_uuid.to_simple_ref());
                    }
                    // FIXME: There might be something to check if the device is
                    // included in the pool, but that is less clear.
                    None
                } else if let Some(mut set) = self.hopeless_device_sets.remove(&pool_uuid) {
                    set.insert(info);
                    self.hopeless_device_sets.insert(pool_uuid, set);
                    None
                } else {
                    let devices = self
                        .errored_pool_devices
                        .remove(&pool_uuid)
                        .unwrap_or_else(HashMap::new);

                    let devices = match self.process_info_add(devices, info) {
                        Err(hopeless) => {
                            self.hopeless_device_sets.insert(pool_uuid, hopeless);
                            return None
                        }
                        Ok(devices) => devices,
                    };

                    // FIXME: An attempt to set up the pool is made, even if no
                    // new device has been added to the set of devices that appear
                    // to belong to the pool. The reason for this is that there
                    // may be many causes of failure to set up a pool, and that
                    // it may be worth another try. If an attempt to setup the
                    // pool is only made on discovery of a new device that may
                    // leave a pool that could be set up in limbo forever. An
                    // alternative, where the user can explicitly ask to try to
                    // set up an incomplete pool would be a better choice.
                    self.try_setup_pool(pools, pool_uuid, devices)
                        .map(|(name, pool)| (pool_uuid, name, pool))
                }
            })
        } else if event_type == libudev::EventType::Remove {
            identify_block_device(event.device()).and_then(move |info| {
                let info: LInfo = info.into();
                let stratis_identifiers = info.stratis_identifiers();
                let pool_uuid = stratis_identifiers.pool_uuid;
                let device_uuid = stratis_identifiers.device_uuid;
                if let Some((_, pool)) = pools.get_by_uuid(pool_uuid) {
                    if pool.get_strat_blockdev(device_uuid).is_some() {
                        warn!("udev reports that a device with {} that appears to belong to a pool with UUID {} has just been removed; this is likely to result in data loss",
                              info,
                              pool_uuid.to_simple_ref());
                    }
                    None
                } else if let Some(mut set) = self.hopeless_device_sets.remove(&pool_uuid) {
                    set.remove(&info);
                    self.hopeless_device_sets.insert(pool_uuid, set);
                    None
                } else {
                    let mut devices = self
                        .errored_pool_devices
                        .remove(&pool_uuid)
                        .unwrap_or_else(HashMap::new);

                    self.process_info_remove(&mut devices, info);

                    self.try_setup_pool(pools, pool_uuid, devices)
                        .map(|(name, pool)| (pool_uuid, name, pool))
                }
            })
        } else {
            None
        }
    }
}

impl<'a> Into<Value> for &'a LiminalDevices {
    fn into(self) -> Value {
        Value::Array(
            self.errored_pool_devices
                .iter()
                .map(|(uuid, map)| {
                    json!({
                        "pool_uuid": uuid.to_simple_ref().to_string(),
                        "devices": Value::Array(map.values().map(|info| info.into()).collect()),
                    })
                })
                .collect(),
        )
    }
}
