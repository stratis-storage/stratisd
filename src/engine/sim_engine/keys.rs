// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    convert::TryFrom,
    fs::File,
    io::{Read, Write},
    os::unix::io::{FromRawFd, RawFd},
};

use termios::Termios;

use devicemapper::Bytes;
use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        types::{DeleteAction, KeyDescription, KeySerial, MappingCreateAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Debug, Default)]
pub struct SimKeyActions(HashMap<KeyDescription, Vec<u8>>);

impl SimKeyActions {
    pub fn contains_key(&self, key_desc: &KeyDescription) -> bool {
        self.0.contains_key(key_desc)
    }

    /// Read the contents of a key from the simulated keyring or return `None`
    /// if no key with the given key description exists.
    fn read(
        &self,
        key_desc: &KeyDescription,
    ) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
        match self.0.get(key_desc) {
            Some(key) => {
                let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
                mem.as_mut().write_all(key)?;
                let key = SizedKeyMemory::new(mem, key.len());
                Ok(Some((0xdead_beef, key)))
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
        interactive: bool,
        handle_term_settings: bool,
    ) -> StratisResult<MappingCreateAction<()>> {
        let key_file = unsafe { File::from_raw_fd(key_fd) };
        let new_key_data = &mut [0u8; MAX_STRATIS_PASS_SIZE];
        let mut bytes_iter = key_file.bytes();

        let old_attrs = if handle_term_settings {
            let old_attrs = Termios::from_fd(key_fd)?;
            let mut new_attrs = old_attrs;
            new_attrs.c_lflag &= !(termios::ICANON | termios::ECHO);
            new_attrs.c_cc[termios::VMIN] = 1;
            new_attrs.c_cc[termios::VTIME] = 0;
            termios::tcsetattr(key_fd, termios::TCSANOW, &new_attrs)?;
            Some(old_attrs)
        } else {
            None
        };

        let mut pos = 0;
        while pos < MAX_STRATIS_PASS_SIZE {
            match bytes_iter.next() {
                Some(Ok(b)) => {
                    if interactive && b as char == '\n' {
                        break;
                    }

                    new_key_data[pos] = b;
                    pos += 1;
                }
                Some(Err(e)) => return Err(e.into()),
                None => break,
            }
        }

        if let Some(ref oa) = old_attrs {
            termios::tcsetattr(key_fd, termios::TCSANOW, oa)?;
        }

        if pos == MAX_STRATIS_PASS_SIZE && bytes_iter.next().is_some() {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "Provided key exceeded maximum allow length of {}",
                    Bytes(MAX_STRATIS_PASS_SIZE as u64)
                ),
            ));
        }

        let key_description = KeyDescription::try_from(key_desc.to_string())?;
        match self.read(&key_description) {
            Ok(Some((_, key_data))) => {
                if key_data.as_ref() == new_key_data as &[u8] {
                    Ok(MappingCreateAction::Identity)
                } else {
                    self.0
                        .insert(key_description.clone(), new_key_data.to_vec());
                    Ok(MappingCreateAction::ValueChanged(()))
                }
            }
            Ok(None) => {
                self.0
                    .insert(key_description.clone(), new_key_data.to_vec());
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
}
