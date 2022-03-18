// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    convert::TryFrom,
    iter::FromIterator,
    path::Path,
    vec::Vec,
};

use serde_json::{Map, Value};

use devicemapper::{Bytes, Sectors, IEC};

use crate::{
    engine::{
        engine::Pool,
        shared::{
            gather_encryption_info, init_cache_idempotent_or_err, validate_filesystem_size_specs,
            validate_name, validate_paths,
        },
        sim_engine::{blockdev::SimDev, filesystem::SimFilesystem},
        structures::Table,
        types::{
            ActionAvailability, BlockDevTier, Clevis, CreateAction, DeleteAction, DevUuid,
            EncryptionInfo, FilesystemUuid, Key, KeyDescription, Name, PoolDiff,
            PoolEncryptionInfo, PoolUuid, RegenAction, RenameAction, SetCreateAction,
            SetDeleteAction,
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
}

impl SimPool {
    pub fn new(paths: &[&Path], enc_info: Option<&EncryptionInfo>) -> (PoolUuid, SimPool) {
        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let device_pairs = devices.iter().map(|p| SimDev::new(p, enc_info));
        (
            PoolUuid::new_v4(),
            SimPool {
                block_devs: device_pairs.collect(),
                cache_devs: HashMap::new(),
                filesystems: Table::default(),
                fs_limit: 10,
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

    fn datadevs_encrypted(&self) -> bool {
        self.encryption_info().is_some()
    }

    #[allow(clippy::unused_self)]
    pub fn destroy(&mut self) -> StratisResult<()> {
        Ok(())
    }

    fn encryption_info(&self) -> Option<PoolEncryptionInfo> {
        gather_encryption_info(
            self.block_devs.len(),
            self.block_devs.iter().map(|(_, bd)| bd.encryption_info()),
        )
        .expect("sim engine cannot create pools with encrypted and unencrypted devices together")
    }

    fn add_clevis_info(&mut self, pin: &str, config: &Value) {
        self.block_devs
            .iter_mut()
            .for_each(|(_, bd)| bd.set_clevis_info(pin, config))
    }

    fn clear_clevis_info(&mut self) {
        self.block_devs
            .iter_mut()
            .for_each(|(_, bd)| bd.unset_clevis_info())
    }

    fn add_key_desc(&mut self, key_desc: &KeyDescription) {
        self.block_devs
            .iter_mut()
            .for_each(|(_, bd)| bd.set_key_desc(key_desc))
    }

    fn clear_key_desc(&mut self) {
        self.block_devs
            .iter_mut()
            .for_each(|(_, bd)| bd.unset_key_desc())
    }

    /// Check the limit of filesystems on a pool and return an error if it has been passed.
    fn check_fs_limit(&self, new_fs: usize) -> StratisResult<()> {
        if convert_int!(self.fs_limit, u64, usize)? < self.filesystems.len() + new_fs {
            Err(StratisError::Msg(format!("The pool limit of {} filesystems has already been reached; increase the filesystem limit on the pool to continue", self.fs_limit)))
        } else {
            Ok(())
        }
    }
}

// Precondition: SimDev::into() always returns a value that matches Value::Object(_).
// Precondition: SimFilesystem::into() always returns a value that matches Value::Object(_).
impl<'a> Into<Value> for &'a SimPool {
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
    type Filesystem = SimFilesystem;
    type BlockDev = SimDev;

    fn init_cache(
        &mut self,
        _pool_uuid: PoolUuid,
        _pool_name: &str,
        blockdevs: &[&Path],
    ) -> StratisResult<SetCreateAction<DevUuid>> {
        if blockdevs.is_empty() {
            return Err(StratisError::Msg(
                "At least one blockdev path is required to initialize a cache.".to_string(),
            ));
        }

        validate_paths(blockdevs)?;

        if self.is_encrypted() {
            return Err(StratisError::Msg(
                "Use of a cache is not supported with an encrypted pool".to_string(),
            ));
        }
        if !self.has_cache() {
            let blockdev_pairs: Vec<_> = blockdevs.iter().map(|p| SimDev::new(p, None)).collect();
            let blockdev_uuids: Vec<_> = blockdev_pairs.iter().map(|(uuid, _)| *uuid).collect();
            self.cache_devs.extend(blockdev_pairs);
            Ok(SetCreateAction::new(blockdev_uuids))
        } else {
            init_cache_idempotent_or_err(
                blockdevs,
                self.cache_devs
                    .iter()
                    .map(|(_, bd)| bd.devnode().to_owned()),
            )
        }
    }

    fn create_filesystems<'a, 'b>(
        &'a mut self,
        _pool_name: &str,
        _pool_uuid: PoolUuid,
        specs: &[(&'b str, Option<Bytes>)],
    ) -> StratisResult<SetCreateAction<(&'b str, FilesystemUuid, Sectors)>> {
        self.check_fs_limit(specs.len())?;

        let spec_map = validate_filesystem_size_specs(specs)?;

        spec_map.iter().fold(Ok(()), |res, (name, size)| {
            res.and_then(|()| validate_name(name))
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
        for (name, size) in spec_map {
            if !self.filesystems.contains_name(name) {
                let uuid = FilesystemUuid::new_v4();
                let new_filesystem = SimFilesystem::new(size);
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
        if tier == BlockDevTier::Cache && !self.has_cache() {
            return Err(StratisError::Msg(
                    "The cache has not been initialized; you must use init_cache first to initialize the cache.".to_string(),
                ));
        }

        if paths.is_empty() {
            // Treat adding no new blockdev as the empty set.
            return Ok((SetCreateAction::new(vec![]), None));
        }

        validate_paths(paths)?;

        let devices: HashSet<_, RandomState> = HashSet::from_iter(paths);
        let encryption_info = pool_enc_to_enc!(self.encryption_info());

        let the_vec = match tier {
            BlockDevTier::Cache => &self.cache_devs,
            BlockDevTier::Data => &self.block_devs,
        };

        let filter: Vec<_> = the_vec.values().map(|d| d.devnode()).collect();

        let filtered_device_pairs: Vec<_> = devices
            .iter()
            .map(|p| {
                SimDev::new(
                    p,
                    match tier {
                        BlockDevTier::Data => encryption_info.as_ref(),
                        BlockDevTier::Cache => None,
                    },
                )
            })
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
        pin: &str,
        clevis_info: &Value,
    ) -> StratisResult<CreateAction<Clevis>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        let clevis_info_current = encryption_info.clevis_info();
        if let Some((current_pin, current_info)) = clevis_info_current {
            if (current_pin.as_str(), current_info) == (pin, clevis_info) {
                Ok(CreateAction::Identity)
            } else {
                Err(StratisError::Msg(format!(
                    "This pool is already bound with clevis pin {} and config {};
                        this differs from the requested pin {} and config {}",
                    current_pin, current_info, pin, clevis_info,
                )))
            }
        } else {
            self.add_clevis_info(pin, clevis_info);
            Ok(CreateAction::Created(Clevis))
        }
    }

    fn unbind_clevis(&mut self) -> StratisResult<DeleteAction<Clevis>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.key_description().is_none() {
            return Err(StratisError::Msg(
                "This device is not bound to a keyring passphrase; refusing to remove \
                the only unlocking method"
                    .to_string(),
            ));
        }

        Ok(if encryption_info.clevis_info().is_some() {
            self.clear_clevis_info();
            DeleteAction::Deleted(Clevis)
        } else {
            DeleteAction::Identity
        })
    }

    fn bind_keyring(
        &mut self,
        key_description: &KeyDescription,
    ) -> StratisResult<CreateAction<Key>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if let Some(kd) = encryption_info.key_description() {
            if key_description == kd {
                Ok(CreateAction::Identity)
            } else {
                Err(StratisError::Msg(format!(
                    "This pool is already bound with key description {};
                        this differs from the requested key description {}",
                    kd.as_application_str(),
                    key_description.as_application_str(),
                )))
            }
        } else {
            self.add_key_desc(key_description);
            Ok(CreateAction::Created(Key))
        }
    }

    fn unbind_keyring(&mut self) -> StratisResult<DeleteAction<Key>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.clevis_info().is_none() {
            return Err(StratisError::Msg(
                "This device is not bound to Clevis; refusing to remove the only \
                unlocking method"
                    .to_string(),
            ));
        }

        Ok(if encryption_info.key_description().is_some() {
            self.clear_key_desc();
            DeleteAction::Deleted(Key)
        } else {
            DeleteAction::Identity
        })
    }

    fn rebind_keyring(
        &mut self,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<RenameAction<Key>> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.key_description().is_none() {
            return Err(StratisError::Msg(
                "This device is not bound to a keyring passphrase; cannot change the passphrase"
                    .to_string(),
            ));
        }

        Ok(if encryption_info.key_description() != Some(new_key_desc) {
            self.add_key_desc(new_key_desc);
            RenameAction::Renamed(Key)
        } else {
            RenameAction::Identity
        })
    }

    // The sim engine does not store token info so this method will always return
    // RenameAction::Identity.
    fn rebind_clevis(&mut self) -> StratisResult<RegenAction> {
        let encryption_info = match pool_enc_to_enc!(self.encryption_info()) {
            Some(ei) => ei,
            None => {
                return Err(StratisError::Msg(
                    "Requested pool does not appear to be encrypted".to_string(),
                ))
            }
        };

        if encryption_info.clevis_info().is_none() {
            Err(StratisError::Msg(
                "This device is not bound to Clevis; cannot regenerate bindings".to_string(),
            ))
        } else {
            Ok(RegenAction)
        }
    }

    fn destroy_filesystems<'a>(
        &'a mut self,
        _pool_name: &str,
        fs_uuids: &[FilesystemUuid],
    ) -> StratisResult<SetDeleteAction<FilesystemUuid>> {
        let mut removed = Vec::new();
        for &uuid in fs_uuids {
            if self.filesystems.remove_by_uuid(uuid).is_some() {
                removed.push(uuid);
            }
        }
        Ok(SetDeleteAction::new(removed))
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
    ) -> StratisResult<CreateAction<(FilesystemUuid, &mut Self::Filesystem)>> {
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
                SimFilesystem::new(filesystem.size())
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

    fn filesystems(&self) -> Vec<(Name, FilesystemUuid, &Self::Filesystem)> {
        self.filesystems
            .iter()
            .map(|(name, uuid, x)| (name.clone(), *uuid, x))
            .collect()
    }

    fn get_filesystem(&self, uuid: FilesystemUuid) -> Option<(Name, &Self::Filesystem)> {
        self.filesystems
            .get_by_uuid(uuid)
            .map(|(name, p)| (name, p))
    }

    fn get_filesystem_by_name(&self, name: &Name) -> Option<(FilesystemUuid, &Self::Filesystem)> {
        self.filesystems
            .get_by_name(name)
            .map(|(uuid, p)| (uuid, p))
    }

    fn blockdevs(&self) -> Vec<(DevUuid, BlockDevTier, &Self::BlockDev)> {
        self.block_devs
            .iter()
            .map(|(uuid, dev)| (uuid, BlockDevTier::Data, dev))
            .chain(
                self.cache_devs
                    .iter()
                    .map(|(uuid, dev)| (uuid, BlockDevTier::Cache, dev)),
            )
            .map(|(uuid, tier, bd)| (*uuid, tier, bd))
            .collect()
    }

    fn get_blockdev(&self, uuid: DevUuid) -> Option<(BlockDevTier, &Self::BlockDev)> {
        self.block_devs
            .get(&uuid)
            .map(|bd| (BlockDevTier::Data, bd))
            .or_else(move || {
                self.cache_devs
                    .get(&uuid)
                    .map(|bd| (BlockDevTier::Cache, bd))
            })
    }

    fn set_blockdev_user_info(
        &mut self,
        _pool_name: &str,
        uuid: DevUuid,
        user_info: Option<&str>,
    ) -> StratisResult<RenameAction<DevUuid>> {
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
        self.datadevs_encrypted()
    }

    fn encryption_info(&self) -> Option<PoolEncryptionInfo> {
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
        true
    }

    fn set_overprov_mode(&mut self, _: &Name, _: bool) -> StratisResult<()> {
        Ok(())
    }

    fn out_of_alloc_space(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {

    use std::path::Path;

    use crate::engine::{
        sim_engine::SimEngine,
        types::{EngineAction, LockKey},
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        let infos = pool
            .create_filesystems(pool_name, uuid, &[("old_name", None)])
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        let results = pool
            .create_filesystems(pool_name, uuid, &[(old_name, None), (new_name, None)])
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        assert!(match pool.destroy_filesystems(pool_name, &[]) {
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        assert_matches!(
            pool.destroy_filesystems(pool_name, &[FilesystemUuid::new_v4()]),
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        let fs_results = pool
            .create_filesystems(pool_name, uuid, &[("fs_name", None)])
            .unwrap()
            .changed()
            .unwrap();
        let fs_uuid = fs_results[0].1;
        assert!(match pool.destroy_filesystems(pool_name, &[fs_uuid]) {
            Ok(filesystems) => filesystems == SetDeleteAction::new(vec![fs_uuid]),
            _ => false,
        });
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        assert!(match pool
            .create_filesystems(pool_name, uuid, &[("name", None)])
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        pool.create_filesystems(pool_name, uuid, &[(fs_name, None)])
            .unwrap();
        let set_create_action = pool
            .create_filesystems(pool_name, uuid, &[(fs_name, None)])
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let mut pool = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        assert!(match pool
            .create_filesystems(pool_name, uuid, &[(fs_name, None), (fs_name, None)])
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
        ))
        .unwrap()
        .changed()
        .unwrap();
        let guard = test_async!(engine.get_mut_pool(LockKey::Uuid(uuid))).unwrap();
        let (pool_name, _, pool) = guard.as_tuple();
        let devices = [Path::new("/s/a"), Path::new("/s/b")];
        assert!(match pool
            .add_blockdevs(uuid, &*pool_name, &devices, BlockDevTier::Data)
            .ok()
            .and_then(|c| c.0.changed())
        {
            Some(devs) => devs.len() == devices.len(),
            _ => false,
        });
    }
}
