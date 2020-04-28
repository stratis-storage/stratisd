// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::io::{FromRawFd, RawFd},
};

use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        types::{CreateAction, DeleteAction, KeySerial, SizedKeyMemory},
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimKeyActions(HashMap<String, String>);

impl SimKeyActions {
    pub fn contains_key(&self, key_desc: &str) -> bool {
        self.0.contains_key(key_desc)
    }
}

impl KeyActions for SimKeyActions {
    fn add(&mut self, key_desc: &str, key_fd: RawFd) -> StratisResult<CreateAction<bool>> {
        let mut line_iter = BufReader::new(unsafe { File::from_raw_fd(key_fd) }).lines();
        let new_key_data = match line_iter.next() {
            Some(Ok(kd)) => kd,
            _ => {
                return Err(StratisError::Error(
                    "Unable to get data passed over the key file descriptor".to_string(),
                ))
            }
        };
        match self.read(key_desc) {
            Ok(Some((_, key_data))) => {
                if key_data.as_ref() == new_key_data.as_bytes() {
                    Ok(CreateAction::Identity)
                } else {
                    self.0.insert(key_desc.to_string(), new_key_data);
                    Ok(CreateAction::Created(true))
                }
            }
            Ok(None) => {
                self.0.insert(key_desc.to_string(), new_key_data);
                Ok(CreateAction::Created(false))
            }
            Err(e) => Err(e),
        }
    }

    fn list(&self) -> StratisResult<Vec<String>> {
        Ok(self.0.keys().cloned().collect())
    }

    fn read(&self, key_desc: &str) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
        match self.0.get(key_desc) {
            Some(key) => {
                let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
                mem.as_mut().write_all(key.as_bytes())?;
                let key = SizedKeyMemory::new(mem, key.len());
                Ok(Some((0xdead_beef, key)))
            }
            None => Ok(None),
        }
    }

    fn delete(&mut self, key_desc: &str) -> StratisResult<DeleteAction<()>> {
        match self.0.remove(key_desc) {
            Some(_) => Ok(DeleteAction::Deleted(())),
            None => Ok(DeleteAction::Identity),
        }
    }
}
