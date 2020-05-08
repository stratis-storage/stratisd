// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Management of devices which are known to stratisd but not in a pool.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::OpenOptions,
    path::PathBuf,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                identify_block_device, CryptHandle, StratBlockDev, StratisIdentifiers, StratisInfo,
                BDA,
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

pub fn convert_to_infos(
    pool_uuid: PoolUuid,
    devnodes: &HashMap<Device, (DevUuid, PathBuf)>,
) -> HashMap<DevUuid, LStratisInfo> {
    devnodes
        .iter()
        .map(|(n, (u, d))| {
            (
                *u,
                LStratisInfo {
                    ids: StratisInfo {
                        identifiers: StratisIdentifiers {
                            pool_uuid,
                            device_uuid: *u,
                        },
                        device_number: *n,
                        devnode: d.to_path_buf(),
                    },
                    luks: None,
                },
            )
        })
        .collect()
}

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

/// Process each element in infos. If the info represents a LUKS device,
/// activate the device. If there is an activation failure, log a warning.
#[allow(dead_code)]
pub fn activate(infos: &HashMap<DevUuid, LInfo>) {
    for (_, info) in infos.iter() {
        if let LInfo::Luks(luks_info) = info {
            let handle = CryptHandle::setup(&luks_info.ids.devnode);
            match handle {
                Err(_) | Ok(None) => {
                    warn!(
                        "Expected device with {} to be a Stratis owned LUKS device but failed to read LUKS metadata for Stratis",
                        luks_info
                    );
                }
                Ok(Some(mut handle)) => {
                    let path = handle.activate();
                    match path {
                        Err(err) => {
                            warn!(
                                "Could not activate Stratis device with {}: {}",
                                luks_info, err
                            );
                        }
                        Ok(_) => {
                            info!("Activated LUKS device with {}", luks_info);
                        }
                    }
                }
            }
        }
    }
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
    #[allow(dead_code)]
    Stratis(LStratisInfo),
    /// A LUKS device
    #[allow(dead_code)]
    Luks(LLuksInfo),
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
#[derive(Debug)]
pub struct LiminalDevices {
    /// Sets of devices which have not been promoted to pools, but which
    /// may still have a chance.
    errored_pool_devices: HashMap<PoolUuid, HashMap<Device, (DevUuid, PathBuf)>>,
    /// Sets of devices which possess some internal contradiction which makes
    /// it impossible for them to be made into sensible pools ever.
    /// Use a HashSet to store the infos for each pool, as the problem that
    /// makes the set hopeless may be duplicate device UUIDs, so that a
    /// map with device UUID keys would not be able to manage all the devices.
    hopeless_device_sets: HashMap<PoolUuid, HashSet<LInfo>>,
}

impl LiminalDevices {
    pub fn new() -> LiminalDevices {
        LiminalDevices {
            errored_pool_devices: HashMap::new(),
            hopeless_device_sets: HashMap::new(),
        }
    }

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

    /// This method is a temporary shim invoked from engine.
    pub fn setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        devices: HashMap<Device, (DevUuid, PathBuf)>,
    ) -> Option<(Name, StratPool)> {
        self.try_setup_pool(pools, pool_uuid, devices)
    }

    /// Given a set of devices, try to set up a pool. If the setup fails,
    /// insert the devices into errored_pool_devices. Otherwise, return the pool.
    /// If there is a name conflict between the set of devices in devices
    /// and some existing pool, return an error.
    ///
    /// Precondition: pools.get_by_uuid(pool_uuid).is_none() &&
    ///               self.errored_pool_devices.get(pool_uuid).is_none()
    ///
    /// Precondition: all devices have already been identified as Stratis
    /// devices. Any encrypted devices have already been unlocked.
    fn try_setup_pool(
        &mut self,
        pools: &Table<StratPool>,
        pool_uuid: PoolUuid,
        devices: HashMap<Device, (DevUuid, PathBuf)>,
    ) -> Option<(Name, StratPool)> {
        assert!(pools.get_by_uuid(pool_uuid).is_none());
        assert!(self.errored_pool_devices.get(&pool_uuid).is_none());

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
            infos: HashMap<DevUuid, LStratisInfo>,
        ) -> Result<Option<(Name, StratPool)>, Destination> {
            let bdas = match get_bdas(&infos) {
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

            let (timestamp, metadata) = match get_metadata(&infos, &bdas) {
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

            let (datadevs, cachedevs) = match get_blockdevs(&metadata.backstore, &infos, bdas) {
                Err(err) => return Err(
                    Destination::Errored(format!(
                        "There was an error encountered when calculating the block devices for pool with UUID {} and name {}: {}",
                        pool_uuid.to_simple_ref(),
                        &metadata.name,
                        err))),
                Ok((datadevs, cachedevs)) => (datadevs, cachedevs),
            };

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

        let result = setup_pool(pools, pool_uuid, convert_to_infos(pool_uuid, &devices));

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
                let infos = devices
                    .iter()
                    .map(|(n, (u, d))| {
                        LInfo::Stratis(LStratisInfo {
                            ids: StratisInfo {
                                identifiers: StratisIdentifiers {
                                    pool_uuid,
                                    device_uuid: *u,
                                },
                                device_number: *n,
                                devnode: d.to_path_buf(),
                            },
                            luks: None,
                        })
                    })
                    .collect();
                self.hopeless_device_sets.insert(pool_uuid, infos);
                None
            }
            Err(Destination::Errored(_)) | Ok(None) => {
                self.errored_pool_devices.insert(pool_uuid, devices);
                None
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
        identify_block_device(event.device()).and_then(move |info| {
            let pool_uuid = info.identifiers.pool_uuid;
            if pools.contains_uuid(pool_uuid) {
                // FIXME: There is the possibilty of an error condition here,
                // if the device found is not in the already set up pool.
                None
            } else if let Some(mut set) = self.hopeless_device_sets.remove(&pool_uuid) {
                set.insert(LInfo::Stratis(LStratisInfo {ids: info, luks: None}));
                self.hopeless_device_sets.insert(pool_uuid, set);
                None
            } else {
                let mut devices = self
                    .errored_pool_devices
                    .remove(&pool_uuid)
                    .unwrap_or_else(HashMap::new);

                if devices
                    .insert(
                        info.device_number,
                        (info.identifiers.device_uuid, info.devnode.to_owned()),
                    )
                    .is_none()
                {
                    info!(
                        "Stratis block device with {} discovered, i.e., identified for the first time during this execution of stratisd",
                        info
                    );
                }

                // FIXME: An attempt to set up the pool is made, even if no
                // new device has been added to the set of devices that appear
                // to belong to the pool. The reason for this is that there
                // may be many causes of failure to set up a pool, and that
                // it may be worth another try. If an attempt to setup the
                // pool is only made on discovery of a new device that may
                // leave a pool that could be set up in limbo forever. An
                // alternative, where the user can explicitly ask to try to
                // set up an incomplete pool would be a better choice.
                self.try_setup_pool(pools, pool_uuid, devices).map(|(name, pool)| (pool_uuid, name, pool))
            }
        })
    }

    /// Generate a JSON report giving some information about the internals
    /// of these devices.
    pub fn report(&self) -> Value {
        Value::Array(self.errored_pool_devices.iter().map(|(uuid, map)| {
            json!({
                "pool_uuid": uuid.to_simple_ref().to_string(),
                "devices": Value::Array(map.values().map(|(_, p)| Value::from(p.display().to_string())).collect()),
            })
        }).collect())
    }
}
