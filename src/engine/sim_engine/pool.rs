// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    path::Path,
    vec::Vec,
};

use either::Either;
use itertools::Itertools;
use serde_json::{Map, Value};

use devicemapper::{Bytes, Sectors, IEC};

use crate::{
    engine::{
        engine::{BlockDev, Filesystem, Pool},
        shared::{
            init_cache_idempotent_or_err, validate_filesystem_size, validate_filesystem_size_specs,
            validate_name, validate_paths,
        },
        sim_engine::{
            blockdev::SimDev, filesystem::SimFilesystem, shared::convert_encryption_info,
        },
        structures::Table,
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptedDevice, EncryptionInfo, FilesystemUuid, GrowAction, InputEncryptionInfo, Key,
            KeyDescription, Name, OptionalTokenSlotInput, PoolDiff, PoolEncryptionInfo, PoolUuid,
            PropChangeAction, RegenAction, RenameAction, SetCreateAction, SetDeleteAction,
            StratSigblockVersion, UnlockMechanism, ValidatedIntegritySpec,
        },
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug)]
pub struct SimPool {
    block_devs: HashMap<DevUuid, SimDev>,
    cache_devs: HashMap<DevUuid, SimDev>,
    filesystems: Table<FilesystemUuid, SimFilesystem>,
    fs_limit: u64,
    enable_overprov: bool,
    encryption_info: Option<EncryptionInfo>,
    integrity_spec: ValidatedIntegritySpec,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
pub struct PoolSave {
    name: String,
    fs_limit: Option<u64>,
    enable_overprov: Option<bool>,
    integrity_spec: Option<ValidatedIntegritySpec>,
}

impl SimPool {
    pub fn new(
        paths: &[&Path],
        enc_info: Option<&EncryptionInfo>,
        integrity_spec: ValidatedIntegritySpec,
    ) -> (PoolUuid, SimPool) {
        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs = devices.iter().map(|p| SimDev::new(p));
        (
            PoolUuid::new_v4(),
            SimPool {
                block_devs: device_pairs.collect(),
                cache_devs: HashMap::new(),
                filesystems: Table::default(),
                fs_limit: 10,
                enable_overprov: true,
                encryption_info: enc_info.cloned(),
                integrity_spec,
            },
        )
    }

    pub fn has_filesystems(&self) -> bool {
        !self.filesystems.is_empty()
    }

    fn get_mut_blockdev_internal(&mut self, uuid: DevUuid) -> Option<(BlockDevTier, &mut SimDev)> {
        let cache_devs = &mut self.cache_devs;
        self.block_devs
            .get_mut(&uuid)
            .map(|bd| (BlockDevTier::Data, bd))
            .or_else(move || {
                cache_devs
                    .get_mut(&uuid)
                    .map(|bd| (BlockDevTier::Cache, bd))
            })
    }

    #[allow(clippy::unused_self)]
    pub fn destroy(&mut self) -> StratisResult<()> {
        Ok(())
    }

    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo> {
        self.encryption_info.as_ref().map(PoolEncryptionInfo::from)
    }

    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>> {
        self.encryption_info.as_ref().cloned().map(Either::Left)
    }

    /// Check the limit of filesystems on a pool and return an error if it has been passed.
    fn check_fs_limit(&self, new_fs: usize) -> StratisResult<()> {
        if convert_int!(self.fs_limit, u64, usize)? < self.filesystems.len() + new_fs {
            Err(StratisError::Msg(format!("The pool limit of {} filesystems has already been reached; increase the filesystem limit on the pool to continue", self.fs_limit)))
        } else {
            Ok(())
        }
    }

    pub fn record(&self, name: &str) -> PoolSave {
        PoolSave {
            name: name.to_owned(),
            enable_overprov: Some(self.enable_overprov),
            fs_limit: Some(self.fs_limit),
            integrity_spec: Some(self.integrity_spec),
        }
    }
}

// Precondition: SimDev::into() always returns a value that matches Value::Object(_).
// Precondition: SimFilesystem::into() always returns a value that matches Value::Object(_).
impl Into<Value> for &SimPool {
    fn into(self) -> Value {
        json!({
            "available_actions": ActionAvailability::Full.to_string(),
            "fs_limit": self.fs_limit,
            "filesystems": Value::Array(
                self.filesystems.iter()
                    .map(|(name, uuid, fs)| {
                        let mut json = Map::new();
                        json.insert("name".to_string(), Value::from(name.to_string()));
                        json.insert("uuid".to_string(), Value::from(uuid.to_string()));
                        if let Value::Object(map) = fs.into() {
                            json.extend(map.into_iter());
                        } else {
                                panic!("SimFilesystem::into() always returns JSON object")
                        }
                        Value::from(json)
                    })
                    .collect()
            ),
            "blockdevs": {
                "datadevs": Value::Array(
                    self.block_devs.iter()
                        .map(|(uuid, dev)| {
                            let mut json = Map::new();
                            json.insert("uuid".to_string(), Value::from(uuid.to_string()));
                            if let Value::Object(map) = dev.into() {
                                json.extend(map.into_iter());
                            } else {
                                panic!("SimDev::into() always returns JSON object")
                            }
                            Value::from(json)
                        })
                        .collect()
                ),
                "cachedevs": Value::Array(
                    self.cache_devs.iter()
                        .map(|(uuid, dev)| {
                            let mut json = Map::new();
                            json.insert("uuid".to_string(), Value::from(uuid.to_string()));
                            if let Value::Object(map) = dev.into() {
                                json.extend(map.into_iter());
                            } else {
                                panic!("SimDev::into() always returns JSON object")
                            }
                            Value::from(json)
                        })
                        .collect()
                ),
            },
        })
    }
}

impl Pool for SimPool {
    fn init_cache(
        &mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        blockdevs: &[&Path],
        supports_encrypted: bool,
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        validate_paths(blockdevs)?;

        if self.is_encrypted() && !supports_encrypted {
            return Err(StratisError::Msg(
                "Use of a cache is not supported with an encrypted pool".to_string(),
            ));
        }
        if !self.has_cache() {
            if blockdevs.is_empty() {
                return Err(StratisError::Msg(
                    "At least one blockdev path is required to initialize a cache.".to_string(),
                ));
            }
            let blockdev_pairs: Vec<_> = blockdevs.iter().map(|p| SimDev::new(p)).collect();
            let blockdev_uuids: Vec<_> = blockdev_pairs.iter().map(|(uuid, _)| *uuid).collect();
            self.cache_devs.extend(blockdev_pairs);
            Ok(SetCreateAction::new(blockdev_uuids))
        } else {
            init_cache_idempotent_or_err(
                blockdevs,
                self.cache_devs.values().map(|bd| bd.devnode().to_owned()),
            )
        }
    }

    fn create_filesystems<'b>(
        &mut self,
        _pool_name: &str,
        _pool_uuid: PoolUuid,
        specs: &[(&'b str, Option<Bytes>, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid, Sectors)>> {
        self.check_fs_limit(specs.len())?;

        let spec_map = validate_filesystem_size_specs(specs)?;

        spec_map.iter().try_fold((), |_, (name, (size, _))| {
            validate_name(name)
                .and_then(|()| {
                    if let Some((_, fs)) = self.filesystems.get_by_name(name) {
                        if fs.size() == *size {
                            Ok(())
                        } else {
                            Err(StratisError::Msg(format!(
                                "Size {} of filesystem {} to be created conflicts with size {} for existing filesystem",
                                size,
                                name,
                                fs.size()
                            )))
                        }
                    } else {
                        Ok(())
                    }
                })
        })?;

        let mut result = Vec::new();
        for (name, (size, size_limit)) in spec_map {
            if !self.filesystems.contains_name(name) {
                let uuid = FilesystemUuid::new_v4();
                let new_filesystem = SimFilesystem::new(size, size_limit, None)?;
                self.filesystems
                    .insert(Name::new((name).to_owned()), uuid, new_filesystem);
                result.push((name, uuid, size));
            }
        }

        Ok(SetCreateAction::new(result))
    }

    fn add_blockdevs(
        &mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        paths: &[&Path],
        tier: BlockDevTier,
    ) -> StratisResult<(SetCreateAction<DevUuid>, Option<PoolDiff>)> {
        validate_paths(paths)?;

        if tier == BlockDevTier::Cache && !self.has_cache() {
            return Err(StratisError::Msg(
                    "The cache has not been initialized; you must use init_cache first to initialize the cache.".to_string(),
                ));
        }

        if paths.is_empty() {
            // Treat adding no new blockdev as the empty set.
            return Ok((SetCreateAction::new(vec![]), None));
        }

        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);

        let the_vec = match tier {
            BlockDevTier::Cache => &self.cache_devs,
            BlockDevTier::Data => &self.block_devs,
        };

        let filter: Vec<_> = the_vec.values().map(|d| d.devnode()).collect();

        let filtered_device_pairs: Vec<_> = devices
            .iter()
            .map(|p| SimDev::new(p))
            .filter(|(_, sd)| !filter.contains(&sd.devnode()))
            .collect();

        let ret_uuids = filtered_device_pairs
            .iter()
            .map(|&(uuid, _)| uuid)
            .collect();

        let the_vec = match tier {
            BlockDevTier::Cache => &mut self.cache_devs,
            BlockDevTier::Data => &mut self.block_devs,
        };

        the_vec.extend(filtered_device_pairs);
        Ok((SetCreateAction::new(ret_uuids), None))
    }

    fn bind_clevis(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<(Clevis, u32)>> {
        let encryption_info = match self.encryption_info.as_mut() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        let token_slot_to_add = match token_slot {
            OptionalTokenSlotInput::Some(t) => {
                if let Some(info) = encryption_info.get_info(t) {
                    match info {
                        UnlockMechanism::KeyDesc(_) => {
                            return Err(StratisError::Msg(format!(
                                "Key slot {t} is already in use by a key description"
                            )));
                        }
                        &UnlockMechanism::ClevisInfo((ref current_pin, ref current_config)) => {
                            if (current_pin.as_str(), current_config) != (pin, clevis_info) {
                                return Err(StratisError::Msg(format!(
                                    "Key slot {t} is already in use by Clevis info ({current_pin}, {current_config}); requested ({pin}, {clevis_info})"
                                )));
                            } else {
                                return Ok(CreateAction::Identity);
                            }
                        }
                    }
                } else {
                    t
                }
            }
            OptionalTokenSlotInput::None => encryption_info.free_token_slot(),
            OptionalTokenSlotInput::Legacy => match encryption_info.single_clevis_info() {
                Some((_, (current_pin, current_config))) => {
                    if (current_pin.as_str(), current_config) == (pin, clevis_info) {
                        return Ok(CreateAction::Identity);
                    } else {
                        return Err(StratisError::Msg(format!("Attempted to bind to Clevis with ({pin}, {clevis_info}); already bound with ({current_pin}, {current_config})")));
                    }
                }
                None => encryption_info.free_token_slot(),
            },
        };

        encryption_info.add_info(
            token_slot_to_add,
            UnlockMechanism::ClevisInfo((pin.to_owned(), clevis_info.to_owned())),
        )?;
        Ok(CreateAction::Created((Clevis, token_slot_to_add)))
    }

    fn bind_keyring(
        &mut self,
        token_slot: OptionalTokenSlotInput,
        key_description: &KeyDescription,
    ) -> StratisResult<CreateAction<(Key, u32)>> {
        let encryption_info = match self.encryption_info.as_mut() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        let token_slot_to_add = match token_slot {
            OptionalTokenSlotInput::Some(t) => {
                if let Some(info) = encryption_info.get_info(t) {
                    match info {
                        UnlockMechanism::KeyDesc(ref key_desc) => {
                            if key_description != key_desc {
                                return Err(StratisError::Msg(format!(
                                    "Key slot {t} is already in use by key description {}; requested {}",
                                    key_desc.as_application_str(),
                                    key_description.as_application_str(),
                                )));
                            } else {
                                return Ok(CreateAction::Identity);
                            }
                        }
                        UnlockMechanism::ClevisInfo(_) => {
                            return Err(StratisError::Msg(format!(
                                "Key slot {t} is already in use by a Clevis token"
                            )));
                        }
                    }
                } else {
                    t
                }
            }
            OptionalTokenSlotInput::None => {
                if encryption_info
                    .all_key_descriptions()
                    .map(|(_, kd)| kd)
                    .contains(key_description)
                {
                    return Ok(CreateAction::Identity);
                } else {
                    encryption_info.free_token_slot()
                }
            }
            OptionalTokenSlotInput::Legacy => match encryption_info.single_key_description() {
                Some((_, kd)) => {
                    if kd == key_description {
                        return Ok(CreateAction::Identity);
                    } else {
                        return Err(StratisError::Msg(format!(
                            "Attempted to bind to key description with {}; already bound with {}",
                            key_description.as_application_str(),
                            kd.as_application_str(),
                        )));
                    }
                }
                None => encryption_info.free_token_slot(),
            },
        };

        encryption_info.add_info(
            token_slot_to_add,
            UnlockMechanism::KeyDesc(key_description.to_owned()),
        )?;
        Ok(CreateAction::Created((Key, token_slot_to_add)))
    }

    fn unbind_keyring(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Key>> {
        let encryption_info = match self.encryption_info.as_mut() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.all_token_slots().count() < 2 {
            return Err(StratisError::Msg(
                "There is one remaining unlock method; removing the key description binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let t_opt = token_slot.or_else(|| encryption_info.single_key_description().map(|(t, _)| t));

        let slot = match t_opt {
            Some(t) => {
                if let Some(UnlockMechanism::ClevisInfo(_)) = encryption_info.get_info(t) {
                    return Err(StratisError::Msg(format!(
                        "Cannot unbind token slot {t} from keyring; bound to Clevis",
                    )));
                };
                encryption_info.remove(t)
            }
            None => false,
        };
        Ok(if slot {
            DeleteAction::Deleted(Key)
        } else {
            DeleteAction::Identity
        })
    }

    fn unbind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<DeleteAction<Clevis>> {
        let encryption_info = match self.encryption_info.as_mut() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.all_token_slots().count() < 2 {
            return Err(StratisError::Msg(
                "There is one remaining unlock method; removing the Clevis binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let t_opt = token_slot.or_else(|| encryption_info.single_clevis_info().map(|(t, _)| t));

        let slot = match t_opt {
            Some(t) => {
                if let Some(UnlockMechanism::KeyDesc(_)) = encryption_info.get_info(t) {
                    return Err(StratisError::Msg(format!(
                        "Cannot unbind token slot {t} from Clevis; bound to keyring",
                    )));
                };
                encryption_info.remove(t)
            }
            None => false,
        };
        Ok(if slot {
            DeleteAction::Deleted(Clevis)
        } else {
            DeleteAction::Identity
        })
    }

    fn rebind_keyring(
        &mut self,
        token_slot: Option<u32>,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        let encryption_info = match self.encryption_info.as_mut() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if let Some((token_slot, info)) = token_slot
            .or_else(|| encryption_info.single_key_description().map(|(t, _)| t))
            .and_then(|t| encryption_info.get_info(t).map(|mech| (t, mech)))
        {
            match info {
                UnlockMechanism::KeyDesc(ref key) => {
                    if key != new_key_desc {
                        encryption_info.set_info(token_slot, UnlockMechanism::KeyDesc(new_key_desc.to_owned()))?;
                        Ok(RenameAction::Renamed(Key))
                    } else {
                        Ok(RenameAction::Identity)
                    }
                }
                UnlockMechanism::ClevisInfo(_) => Err(StratisError::Msg(format!(
                    "Cannot rebind slot {token_slot} with a key description; slot is bound to a Clevis config"
                ))),
            }
        } else {
            Err(StratisError::Msg("Cannot rebind empty slot".to_string()))
        }
    }

    // The sim engine does not store token info so this method will always return
    // RenameAction::Identity.
    fn rebind_clevis(&mut self, token_slot: Option<u32>) -> StratisResult<RegenAction> {
        let encryption_info = match self.encryption_info.as_ref() {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if let Some((token_slot, info)) = token_slot
            .or_else(|| encryption_info.single_clevis_info().map(|(t, _)| t))
            .and_then(|t| encryption_info.get_info(t).map(|mech| (t, mech)))
        {
            match info {
                UnlockMechanism::KeyDesc(_) => Err(StratisError::Msg(format!(
                    "Cannot rebind slot {token_slot} with a key description; slot is bound to a key config"
                ))),
                UnlockMechanism::ClevisInfo(_) => Ok(RegenAction),
            }
        } else {
            Err(StratisError::Msg("Cannot rebind empty slot".to_string()))
        }
    }

    fn destroy_filesystems(
        &mut self,
        _pool_name: &str,
        fs_uuids: &HashSet<FilesystemUuid>,
    ) -> StratisResult<SetDeleteAction<FilesystemUuid, (FilesystemUuid, Option<FilesystemUuid>)>>
    {
        let mut snapshots = self
            .filesystems()
            .iter()
            .filter_map(|(_, u, fs)| {
                fs.origin().and_then(|x| {
                    if fs_uuids.contains(&x) {
                        Some((x, (*u, fs.merge_scheduled())))
                    } else {
                        None
                    }
                })
            })
            .fold(HashMap::new(), |mut acc, (u, v)| {
                acc.entry(u)
                    .and_modify(|e: &mut Vec<_>| e.push(v))
                    .or_insert(vec![v]);
                acc
            });

        let scheduled_for_merge = snapshots
            .iter()
            .filter(|(_, snaps)| snaps.iter().any(|(_, scheduled)| *scheduled))
            .collect::<Vec<_>>();
        if !scheduled_for_merge.is_empty() {
            let err_str = format!("The filesystem destroy operation can not be begun until the revert operations for the following filesystem snapshots have been cancelled: {}", scheduled_for_merge.iter().map(|(u, _)| u.to_string()).collect::<Vec<_>>().join(", "));
            return Err(StratisError::Msg(err_str));
        }

        let (mut removed, mut updated_origins) = (Vec::new(), Vec::new());
        for &uuid in fs_uuids {
            if let Some((_, fs)) = self.get_filesystem(uuid) {
                let fs_origin = fs.origin();
                self.filesystems
                    .remove_by_uuid(uuid)
                    .expect("just looked up");
                removed.push(uuid);

                for (sn_uuid, _) in snapshots.remove(&uuid).unwrap_or_else(Vec::new) {
                    // The filesystems may have been removed; any one of
                    // them may also be a filesystem that was scheduled for
                    // removal.
                    if let Some((_, sn)) = self.filesystems.get_mut_by_uuid(sn_uuid) {
                        assert!(
                            sn.set_origin(fs_origin),
                            "A snapshot can only have one origin, so it can be in snapshots.values() only once, so its origin value can be set only once"
                        );
                        updated_origins.push((sn_uuid, fs_origin));
                    };
                }
            }
        }

        Ok(SetDeleteAction::new(removed, updated_origins))
    }

    fn rename_filesystem(
        &mut self,
        _pool_name: &str,
        uuid: FilesystemUuid,
        new_name: &str,
    ) -> StratisResult<RenameAction<FilesystemUuid>> {
        validate_name(new_name)?;

        rename_filesystem_pre_idem!(self; uuid; new_name);

        let (_, filesystem) = self
            .filesystems
            .remove_by_uuid(uuid)
            .expect("Must succeed since self.filesystems.get_by_uuid() returned a value");

        self.filesystems
            .insert(Name::new(new_name.to_owned()), uuid, filesystem);

        Ok(RenameAction::Renamed(uuid))
    }

    fn snapshot_filesystem(
        &mut self,
        _pool_name: &str,
        _pool_uuid: PoolUuid,
        origin_uuid: FilesystemUuid,
        snapshot_name: &str,
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut dyn Filesystem)>> {
        self.check_fs_limit(1)?;

        validate_name(snapshot_name)?;

        let target = self.filesystems.get_by_name(snapshot_name);

        let snapshot = match self.filesystems.get_by_uuid(origin_uuid) {
            Some((_, filesystem)) => {
                if let Some((_, target_fs)) = target {
                    if target_fs.size() != filesystem.size() {
                        return Err(StratisError::Msg(format!(
                                    "Filesystem {} already exists and has size {} different from source filesystem size {}",
                                    snapshot_name,
                                    target_fs.size(),
                                    filesystem.size())));
                    } else {
                        return Ok(CreateAction::Identity);
                    }
                }
                SimFilesystem::new(
                    filesystem.size(),
                    filesystem.size_limit(),
                    Some(origin_uuid),
                )?
            }
            None => {
                return Err(StratisError::Msg(origin_uuid.to_string()));
            }
        };

        let uuid = FilesystemUuid::new_v4();
        self.filesystems
            .insert(Name::new(snapshot_name.to_owned()), uuid, snapshot);
        Ok(CreateAction::Created((
            uuid,
            self.filesystems
                .get_mut_by_uuid(uuid)
                .expect("just inserted")
                .1,
        )))
    }

    fn total_physical_size(&self) -> Sectors {
        // We choose to make our pools very big, and we can change that
        // if it is inconvenient.
        Sectors(IEC::Ei)
    }

    fn total_allocated_size(&self) -> Sectors {
        Sectors(5 * IEC::Mi)
    }

    fn total_physical_used(&self) -> Option<Sectors> {
        Some(Sectors(0))
    }

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &dyn Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x as &dyn Filesystem))
            .collect()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &dyn Filesystem)> {
        self.filesystems
            .get_by_uuid(uuid)
            .map(|(name, p)| (name, p as &dyn Filesystem))
    }

    fn get_filesystem_by_name(&self, name: &Name) -> Option<(FilesystemUuid, &dyn Filesystem)> {
        self.filesystems
            .get_by_name(name)
            .map(|(uuid, p)| (uuid, p as &dyn Filesystem))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &dyn BlockDev)> {
        self.block_devs
            .iter()
            .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev))
            .chain(
                self.cache_devs
                    .iter()
                    .map(|(uuid, dev)| (uuid, BlockDevTier::Cache, dev)),
            )
            .map(|(uuid, tier, bd)| (*uuid, tier, bd as &dyn BlockDev))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &dyn BlockDev)> {
        self.block_devs
            .get(&uuid)
            .map(|bd| (BlockDevTier::Data, bd as &dyn BlockDev))
            .or_else(move || {
                self.cache_devs
                    .get(&uuid)
                    .map(|bd| (BlockDevTier::Cache, bd as &dyn BlockDev))
            })
    }

    fn get_mut_blockdev(
        &mut self,
        uuid: DevUuid,
    ) -> StratisResult<Option<(BlockDevTier, &mut dyn BlockDev)>> {
        Ok(match self.block_devs.get_mut(&uuid) {
            Some(bd) => Some((BlockDevTier::Data, bd as &mut dyn BlockDev)),
            None => self
                .cache_devs
                .get_mut(&uuid)
                .map(|bd| (BlockDevTier::Cache, bd as &mut dyn BlockDev)),
        })
    }

    fn set_blockdev_user_info(
        &mut self,
        _pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>> {
        user_info.map(validate_name).transpose()?;
        Ok(self.get_mut_blockdev_internal(uuid).map_or_else(
            || RenameAction::NoSource,
            |(_, b)| {
                if b.set_user_info(user_info) {
                    RenameAction::Renamed(uuid)
                } else {
                    RenameAction::Identity
                }
            },
        ))
    }

    fn has_cache(&self) -> bool {
        !self.cache_devs.is_empty()
    }

    fn is_encrypted(&self) -> bool {
        self.encryption_info().is_some()
    }

    fn encryption_info_legacy(&self) -> Option<PoolEncryptionInfo> {
        self.encryption_info_legacy()
    }

    fn encryption_info(&self) -> Option<Either<EncryptionInfo, PoolEncryptionInfo>> {
        self.encryption_info()
    }

    fn avail_actions(&self) -> ActionAvailability {
        ActionAvailability::Full
    }

    fn fs_limit(&self) -> u64 {
        self.fs_limit
    }

    fn set_fs_limit(&mut self, _: &Name, _: PoolUuid, new_limit: u64) -> StratisResult<()> {
        if new_limit <= self.fs_limit {
            Err(StratisError::Msg(
                "New filesystem limit must be greater than old limit".to_string(),
            ))
        } else {
            self.fs_limit = new_limit;
            Ok(())
        }
    }

    fn overprov_enabled(&self) -> bool {
        self.enable_overprov
    }

    fn set_overprov_mode(&mut self, _: &Name, enabled: bool) -> StratisResult<()> {
        self.enable_overprov = enabled;
        Ok(())
    }

    fn out_of_alloc_space(&self) -> bool {
        false
    }

    fn grow_physical(
        &mut self,
        _: &Name,
        _: PoolUuid,
        _: DevUuid,
    ) -> StratisResult<(GrowAction<(PoolUuid, DevUuid)>, Option<PoolDiff>)> {
        Ok((GrowAction::Identity, None))
    }

    fn set_fs_size_limit(
        &mut self,
        fs_uuid: FilesystemUuid,
        limit: Option<Bytes>,
    ) -> StratisResult<PropChangeAction<Option<Sectors>>> {
        let (name, fs) = self.filesystems.get_mut_by_uuid(fs_uuid).ok_or_else(|| {
            StratisError::Msg(format!("Filesystem with UUID {fs_uuid} not found"))
        })?;
        let limit = validate_filesystem_size(&name, limit)?;
        let changed = fs.set_size_limit(limit)?;
        if changed {
            Ok(PropChangeAction::NewValue(limit))
        } else {
            Ok(PropChangeAction::Identity)
        }
    }

    fn encrypt_pool(
        &mut self,
        _: &Name,
        _: PoolUuid,
        enc: &InputEncryptionInfo,
    ) -> StratisResult<CreateAction<EncryptedDevice>> {
        self.encryption_info = convert_encryption_info(Some(enc), None)?;
        Ok(CreateAction::Created(EncryptedDevice))
    }

    fn current_metadata(&self, pool_name: &Name) -> StratisResult<String> {
        serde_json::to_string(&self.record(pool_name)).map_err(|e| e.into())
    }

    fn last_metadata(&self) -> StratisResult<String> {
        // Just invent a name for the pool; a sim pool has no real metadata
        serde_json::to_string(&self.record("<name>")).map_err(|e| e.into())
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        StratSigblockVersion::V2
    }

    fn current_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        serde_json::to_string(
            &self
                .filesystems
                .iter()
                .filter_map(|(name, uuid, fs)| {
                    if fs_name.map(|n| *n == **name).unwrap_or(true) {
                        Some((uuid, fs.record(name, *uuid)))
                    } else {
                        None
                    }
                })
                .collect::<HashMap<_, _>>(),
        )
        .map_err(|e| e.into())
    }

    fn last_fs_metadata(&self, fs_name: Option<&str>) -> StratisResult<String> {
        // The sim pool doesn't write data, so the last fs metadata and the
        // current fs metadata are, by definition, the same.
        self.current_fs_metadata(fs_name)
    }

    fn set_fs_merge_scheduled(
        &mut self,
        fs_uuid: FilesystemUuid,
        scheduled: bool,
    ) -> StratisResult<PropChangeAction<bool>> {
        let (_, fs) = self
            .filesystems
            .get_by_uuid(fs_uuid)
            .ok_or_else(|| StratisError::Msg(format!("No filesystem with UUID {fs_uuid} found")))?;

        let origin = fs.origin().ok_or_else(|| {
            StratisError::Msg(format!(
                "Filesystem {fs_uuid} has no origin, revert cannot be scheduled"
            ))
        })?;

        if fs.merge_scheduled() == scheduled {
            return Ok(PropChangeAction::Identity);
        }

        if scheduled {
            if self
                .filesystems
                .get_by_uuid(origin)
                .map(|(_, fs)| fs.merge_scheduled())
                .unwrap_or(false)
            {
                return Err(StratisError::Msg(format!(
                    "Filesystem {fs_uuid} is scheduled to replace filesystem {origin}, but filesystem {origin} is already scheduled to replace another filesystem. Since the order in which the filesystems should replace each other is unknown, this operation can not be performed."
                )));
            }

            let (others_scheduled, into_scheduled) = self.filesystems.iter().fold(
                (Vec::new(), Vec::new()),
                |(mut o_s, mut i_s), (u, n, f)| {
                    if f.origin().map(|o| o == origin).unwrap_or(false) && f.merge_scheduled() {
                        o_s.push((u, n, f));
                    }
                    if f.origin().map(|o| o == fs_uuid).unwrap_or(false) && f.merge_scheduled() {
                        i_s.push((u, n, f));
                    }
                    (o_s, i_s)
                },
            );

            if let Some((n, u, _)) = others_scheduled.first() {
                return Err(StratisError::Msg(format!(
                    "Filesystem {n} with UUID {u} is already scheduled to be reverted into origin filesystem {origin}"
                )));
            }

            if let Some((n, u, _)) = into_scheduled.first() {
                return Err(StratisError::Msg(format!(
                    "Filesystem {n} with UUID {u} is already scheduled to be reverted into this filesystem {origin}. The ordering is ambiguous, unwilling to schedule a revert"
                )));
            }
        }

        assert!(
            self.filesystems
                .get_mut_by_uuid(fs_uuid)
                .expect("Looked up above")
                .1
                .set_merge_scheduled(scheduled)
                .expect("fs.origin() is not None"),
            "Already returned from this method if value to set is the same as current"
        );

        Ok(PropChangeAction::NewValue(scheduled))
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;

    use crate::engine::{
        sim_engine::SimEngine,
        types::{EngineAction, IntegritySpec, PoolIdentifier},
        Engine,
    };

    use super::*;

    #[test]
    /// Renaming a filesystem on an empty pool always works
    fn rename_empty() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert_matches!(
            pool.rename_filesystem(pool_name, FilesystemUuid::new_v4(), "new_name"),
            Ok(RenameAction::NoSource)
        );
    }

    #[test]
    /// Renaming a filesystem to another filesystem should work if new name not taken
    fn rename_happens() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        let infos = pool
            .create_filesystems(pool_name, uuid, &[("old_name", None, None)])
            .unwrap()
            .changed()
            .unwrap();
        assert_matches!(
            pool.rename_filesystem(pool_name, infos[0].1, "new_name")
                .unwrap(),
            RenameAction::Renamed(_)
        );
    }

    #[test]
    /// Renaming a filesystem to another filesystem should fail if new name taken
    fn rename_fails() {
        let old_name = "old_name";
        let new_name = "new_name";
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        let results = pool
            .create_filesystems(
                pool_name,
                uuid,
                &[(old_name, None, None), (new_name, None, None)],
            )
            .unwrap()
            .changed()
            .unwrap();
        let old_uuid = results.iter().find(|x| x.0 == old_name).unwrap().1;
        assert_matches!(
            pool.rename_filesystem(pool_name, old_uuid, new_name),
            Err(_)
        );
    }

    #[test]
    /// Renaming should succeed if old_name absent, new present
    fn rename_no_op() {
        let new_name = "new_name";
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert_matches!(
            pool.rename_filesystem(pool_name, FilesystemUuid::new_v4(), new_name),
            Ok(RenameAction::NoSource)
        );
    }

    #[test]
    /// Removing an empty list of filesystems should always succeed
    fn destroy_fs_empty() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert!(match pool.destroy_filesystems(pool_name, &HashSet::new()) {
            Ok(uuids) => !uuids.is_changed(),
            _ => false,
        });
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on empty pool
    fn destroy_fs_some() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert_matches!(
            pool.destroy_filesystems(pool_name, &[FilesystemUuid::new_v4()].into()),
            Ok(_)
        );
    }

    #[test]
    /// Removing a non-empty list of filesystems should succeed on any pool
    fn destroy_fs_any() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        let fs_results = pool
            .create_filesystems(pool_name, uuid, &[("fs_name", None, None)])
            .unwrap()
            .changed()
            .unwrap();
        let fs_uuid = fs_results[0].1;
        assert!(
            match pool.destroy_filesystems(pool_name, &[fs_uuid].into()) {
                Ok(filesystems) => filesystems == SetDeleteAction::new(vec![fs_uuid], vec![]),
                _ => false,
            }
        );
    }

    #[test]
    /// Creating an empty list of filesystems should succeed, always
    fn create_fs_none() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        let fs = pool.create_filesystems(pool_name, uuid, &[]).unwrap();
        assert!(!fs.is_changed())
    }

    #[test]
    /// Creating a non-empty list of filesystems always succeeds.
    fn create_fs_some() {
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert!(match pool
            .create_filesystems(pool_name, uuid, &[("name", None, None)])
            .ok()
            .and_then(|fs| fs.changed())
        {
            Some(names) => (names.len() == 1) & (names[0].0 == "name"),
            _ => false,
        });
    }

    #[test]
    /// Creating a an already existing filesystem fails.
    fn create_fs_conflict() {
        let fs_name = "fs_name";
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        pool.create_filesystems(pool_name, uuid, &[(fs_name, None, None)])
            .unwrap();
        let set_create_action = pool
            .create_filesystems(pool_name, uuid, &[(fs_name, None, None)])
            .unwrap();
        assert!(!set_create_action.is_changed());
    }

    #[test]
    /// Requesting identical filesystems succeeds.
    fn create_fs_dups() {
        let fs_name = "fs_name";
        let engine = SimEngine::default();
        let pool_name = "pool_name";
        let uuid = test_async!(engine.create_pool(
            pool_name,
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        assert!(match pool
            .create_filesystems(
                pool_name,
                uuid,
                &[(fs_name, None, None), (fs_name, None, None)]
            )
            .ok()
            .and_then(|fs| fs.changed())
        {
            Some(names) => (names.len() == 1) & (names[0].0 == fs_name),
            _ => false,
        });
    }

    #[test]
    /// Adding a list of devices to an empty pool should yield list.
    fn add_device_empty() {
        let engine = SimEngine::default();
        let uuid = test_async!(engine.create_pool(
            "pool_name",
            strs_to_paths!(["/dev/one", "/dev/two", "/dev/three"]),
            None,
            IntegritySpec::default(),
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut guard = test_async!(engine.get_mut_pool(PoolIdentifier::Uuid(uuid))).unwrap();
        let (pool_name, _, pool) = guard.as_mut_tuple();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(match pool
            .add_blockdevs(uuid, &pool_name, &devices, BlockDevTier::Data)
            .ok()
            .and_then(|c| c.0.changed())
        {
            Some(devs) => devs.len() == devices.len(),
            _ => false,
        });
    }
}
