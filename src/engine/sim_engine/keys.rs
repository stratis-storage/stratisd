// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, os::unix::io::RawFd};

use crate::{
    engine::{
        engine::KeyActions,
        shared,
        types::{Key, KeyDescription, MappingCreateAction, MappingDeleteAction},
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
    fn read(&self, key_desc: &KeyDescription) -> Option<&[u8]> {
        self.0.get(key_desc).map(|mem| mem.as_slice())
    }
}

impl KeyActions for SimKeyActions {
    fn set(
        &mut self,
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<Key>> {
        let mut memory = Vec::new();
        let _ = shared::set_key_shared(key_fd, &mut memory)?;

        match self.read(key_desc) {
            Some(key_data) => {
                if key_data == memory.as_slice() {
                    Ok(MappingCreateAction::Identity)
                } else {
                    self.0.insert((*key_desc).clone(), memory);
                    Ok(MappingCreateAction::ValueChanged(Key))
                }
            }
            None => {
                self.0.insert((*key_desc).clone(), memory);
                Ok(MappingCreateAction::Created(Key))
            }
        }
    }

    fn list(&self) -> StratisResult<Vec<KeyDescription>> {
        Ok(self.0.keys().cloned().collect())
    }

    fn unset(&mut self, key_desc: &KeyDescription) -> StratisResult<MappingDeleteAction<Key>> {
        match self.0.remove(key_desc) {
            Some(_) => Ok(MappingDeleteAction::Deleted(Key)),
            None => Ok(MappingDeleteAction::Identity),
        }
    }
}
