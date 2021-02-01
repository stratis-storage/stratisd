// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, io::Write, os::unix::io::RawFd};

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
pub struct SimKeyActions(HashMap<KeyDescription, Vec<u8>>);

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
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<()>> {
        let mut memory = Vec::with_capacity(MAX_STRATIS_PASS_SIZE);
        let len = shared::set_key_shared(key_fd, &mut memory)?;
        memory.truncate(len);

        match self.read(key_desc) {
            Ok(Some(key_data)) => {
                if key_data.as_ref() == memory.as_slice() {
                    Ok(MappingCreateAction::Identity)
                } else {
                    self.0.insert((*key_desc).clone(), memory);
                    Ok(MappingCreateAction::ValueChanged(()))
                }
            }
            Ok(None) => {
                self.0.insert((*key_desc).clone(), memory);
                Ok(MappingCreateAction::Created(()))
            }
            Err(e) => Err(e),
        }
    }

    fn list(&self) -> StratisResult<Vec<KeyDescription>> {
        Ok(self.0.keys().cloned().collect())
    }

    fn unset(&mut self, key_desc: &KeyDescription) -> StratisResult<DeleteAction<()>> {
        match self.0.remove(key_desc) {
            Some(_) => Ok(DeleteAction::Deleted(())),
            None => Ok(DeleteAction::Identity),
        }
    }
}
