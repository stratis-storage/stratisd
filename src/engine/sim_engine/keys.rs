// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, convert::TryFrom, io::Write, os::unix::io::RawFd};

use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        shared,
        types::{DeleteAction, KeyDescription, MappingCreateAction, SizedKeyMemory},
    },
    stratis::StratisResult,
};

#[derive(Debug, Default)]
pub struct SimKeyActions(HashMap<KeyDescription, SizedKeyMemory>);

impl SimKeyActions {
    pub fn contains_key(&self, key_desc: &KeyDescription) -> bool {
        self.0.contains_key(key_desc)
    }

    /// Read the contents of a key from the simulated keyring or return `None`
    /// if no key with the given key description exists.
    fn read(&self, key_desc: &KeyDescription) -> StratisResult<Option<SizedKeyMemory>> {
        match self.0.get(key_desc) {
            Some(key) => {
                let mut key_clone = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
                let size = key_clone.as_mut().write(key.as_ref())?;
                Ok(Some(SizedKeyMemory::new(key_clone, size)))
            }
            None => Ok(None),
        }
    }
}

impl KeyActions for SimKeyActions {
    fn set(
        &mut self,
        key_desc: &str,
        key_fd: RawFd,
        interactive: Option<bool>,
    ) -> StratisResult<MappingCreateAction<()>> {
        let memory = shared::set_key_shared(key_fd, interactive)?;

        let key_description = KeyDescription::try_from(key_desc.to_string())?;
        match self.read(&key_description) {
            Ok(Some(key_data)) => {
                if key_data.as_ref() == memory.as_ref() {
                    Ok(MappingCreateAction::Identity)
                } else {
                    self.0.insert(key_description.clone(), memory);
                    Ok(MappingCreateAction::ValueChanged(()))
                }
            }
            Ok(None) => {
                self.0.insert(key_description.clone(), memory);
                Ok(MappingCreateAction::Created(()))
            }
            Err(e) => Err(e),
        }
    }

    fn list(&self) -> StratisResult<Vec<KeyDescription>> {
        Ok(self.0.keys().cloned().collect())
    }

    fn unset(&mut self, key_desc: &str) -> StratisResult<DeleteAction<()>> {
        let key_description = KeyDescription::try_from(key_desc.to_string())?;
        match self.0.remove(&key_description) {
            Some(_) => Ok(DeleteAction::Deleted(())),
            None => Ok(DeleteAction::Identity),
        }
    }

    fn expiration(&self) -> StratisResult<String> {
        Ok("No expiration".to_string())
    }
}
