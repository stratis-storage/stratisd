// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ffi::CString,
    fs::File,
    io::{self, Read},
    iter::Take,
    os::unix::io::{FromRawFd, RawFd},
    slice::Iter,
    str,
};

use libc::{syscall, SYS_add_key, SYS_keyctl};
use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        strat_engine::names::KeyDescription,
        types::{DeleteAction, KeySerial, MappingCreateAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// This value indicates the maximum number of keys that can be listed at one time.
/// This is an implementation decision and can be increased if the ability to list
/// more keys is desired.
const MAX_NUM_KEY_IDS: usize = 4096;
/// This value indicates the maximum accepted length in bytes of a `KEYCTL_DESCRIBE`
/// string returned when querying the kernel.
const MAX_KEYCTL_DESCRIBE_STRING_LEN: usize = 4096;

/// Get the ID of the persistent root user keyring and attach it to
/// the session keyring.
fn get_persistent_keyring() -> StratisResult<KeySerial> {
    // Attach persistent keyring to session keyring
    match unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_GET_PERSISTENT,
            0,
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        i => Ok(i as KeySerial),
    }
}

/// Search for the given key description in the persistent root keyring.
/// Returns the key ID or nothing if it was not found in the keyring.
fn search_key(
    keyring_id: KeySerial,
    key_desc: &KeyDescription,
) -> StratisResult<Option<KeySerial>> {
    let key_desc_cstring = CString::new(key_desc.to_system_string()).map_err(|_| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            "Invalid key description provided".to_string(),
        )
    })?;

    let key_id = unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_SEARCH,
            keyring_id,
            concat!("user", "\0").as_ptr(),
            key_desc_cstring.as_ptr(),
            0,
        )
    };
    if key_id < 0 {
        if unsafe { *libc::__errno_location() } == libc::ENOKEY {
            Ok(None)
        } else {
            Err(io::Error::last_os_error().into())
        }
    } else {
        Ok(Some(key_id as KeySerial))
    }
}

/// Read a key with the provided key description into safely handled memory if it
/// exists in the keyring.
///
/// The return type with be a tuple of an `Option` and a keyring id. The `Option`
/// type will be `Some` if the key was found in the keyring and will contain
/// the key ID and the key contents. If no key was found with the provided
/// key description, `None` will be returned.
pub fn read_key(
    key_desc: &KeyDescription,
) -> StratisResult<(Option<(KeySerial, SizedKeyMemory)>, KeySerial)> {
    let keyring_id = get_persistent_keyring()?;

    let key_id_option = search_key(keyring_id, key_desc)?;
    let key_id = if let Some(ki) = key_id_option {
        ki
    } else {
        return Ok((None, keyring_id));
    };

    let mut key_buffer = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
    let mut_ref = key_buffer.as_mut();

    // Read key from keyring
    match unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_READ,
            key_id,
            mut_ref.as_mut_ptr(),
            mut_ref.len(),
        )
    } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        i => Ok((
            Some((
                key_id as KeySerial,
                SizedKeyMemory::new(key_buffer, i as usize),
            )),
            keyring_id,
        )),
    }
}

/// Reset the key data attached to the provided key description if the new key data
/// is different from the old key data.
// Precondition: The key description is already present in the keyring.
fn reset_key(
    key_id: KeySerial,
    old_key_data: SizedKeyMemory,
    new_key_data: SizedKeyMemory,
) -> StratisResult<bool> {
    if old_key_data.as_ref() == new_key_data.as_ref() {
        Ok(false)
    } else {
        // Update the existing key data
        let update_result = unsafe {
            syscall(
                SYS_keyctl,
                libc::KEYCTL_UPDATE,
                key_id,
                new_key_data.as_ref().as_ptr(),
                new_key_data.as_ref().len(),
            )
        };
        if update_result < 0 {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(true)
        }
    }
}

/// Add the key to the given keyring attaching it to the provided key description.
// Precondition: The key description was not already present.
fn set_key(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
    keyring_id: KeySerial,
) -> StratisResult<()> {
    let key_desc_cstring = CString::new(key_desc.to_system_string()).map_err(|_| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            "Invalid key description provided".to_string(),
        )
    })?;
    // Add a key to the kernel keyring
    if unsafe {
        libc::syscall(
            SYS_add_key,
            concat!("user", "\0").as_ptr(),
            key_desc_cstring.as_ptr(),
            key_data.as_ref().as_ptr(),
            key_data.as_ref().len(),
            keyring_id,
        )
    } < 0
    {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

/// Perform an idempotent add of the given key data with the given key description.
///
/// Successful return values:
/// * `Ok(MappingCreateAction::Identity)`: The key was already in the keyring with the
/// appropriate key description and key data.
/// * `Ok(MappingCreateAction::Created(()))`: The key was newly added to the keyring.
/// * `Ok(MappingCreateAction::Changed)`: The key description was already present
/// in the keyring but the key data was updated.
fn set_key_idem(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
) -> StratisResult<MappingCreateAction<()>> {
    match read_key(key_desc) {
        Ok((Some((key_id, old_key_data)), _)) => {
            let changed = reset_key(key_id, old_key_data, key_data)?;
            if changed {
                Ok(MappingCreateAction::ValueChanged)
            } else {
                Ok(MappingCreateAction::Identity)
            }
        }
        Ok((None, keyring_id)) => {
            set_key(key_desc, key_data, keyring_id)?;
            Ok(MappingCreateAction::Created(()))
        }
        Err(e) => Err(e),
    }
}

/// Parse the returned key string from `KEYCTL_DESCRIBE` into a key description.
fn parse_keyctl_describe_string(key_str: &str) -> StratisResult<String> {
    key_str
        .rsplit(';')
        .next()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::Invalid,
                "Invalid format returned from the kernel query for the key description".to_string(),
            )
        })
}

/// A list of key IDs that were read from the persistent root keyring.
///
/// This list must keep track of the size externally because the buffer must be
/// allocated as the maximum allowable size before it is coerced down to
/// a pointer to use it in a syscall.
struct KeyIdList {
    key_ids: [KeySerial; MAX_NUM_KEY_IDS],
    num_key_ids: usize,
}

impl KeyIdList {
    /// Create a new list of key IDs.
    fn new() -> KeyIdList {
        KeyIdList {
            key_ids: [0; MAX_NUM_KEY_IDS],
            num_key_ids: 0,
        }
    }

    /// Populate the list with IDs from the persistent root kernel keyring.
    fn populate(&mut self) -> StratisResult<()> {
        let keyring_id = get_persistent_keyring()?;

        // Read list of keys in the persistent keyring.
        match unsafe {
            syscall(
                SYS_keyctl,
                libc::KEYCTL_READ,
                keyring_id,
                self.key_ids.as_mut_ptr(),
                self.key_ids.len(),
            )
        } {
            i if i < 0 => return Err(io::Error::last_os_error().into()),
            i => {
                let ret = i as usize;
                let num_key_ids = if ret > MAX_NUM_KEY_IDS {
                    warn!(
                        "Some key entries were truncated. Stratis can only list \
                        a maximum of {} keys.",
                        MAX_NUM_KEY_IDS
                    );
                    MAX_NUM_KEY_IDS
                } else {
                    ret
                };
                self.num_key_ids = num_key_ids;
            }
        };
        Ok(())
    }

    /// Get the number of key IDs currently stored in this list.
    fn len(&self) -> usize {
        self.num_key_ids
    }

    /// Iterate through the key IDs.
    fn iter(&self) -> Take<Iter<KeySerial>> {
        let len = self.len();
        self.key_ids.iter().take(len)
    }

    /// Get the list of key descriptions corresponding to the kernel key IDs.
    fn to_key_descs(&self) -> StratisResult<Vec<String>> {
        let mut key_descs = Vec::new();
        let mut keyctl_buffer = [0u8; MAX_KEYCTL_DESCRIBE_STRING_LEN];
        for id in self.iter() {
            let len = match unsafe {
                syscall(
                    SYS_keyctl,
                    libc::KEYCTL_DESCRIBE,
                    id,
                    keyctl_buffer.as_mut_ptr(),
                    keyctl_buffer.len(),
                )
            } {
                i if i < 0 => return Err(io::Error::last_os_error().into()),
                i => {
                    let len = i as usize;
                    if len > MAX_KEYCTL_DESCRIBE_STRING_LEN {
                        warn!(
                            "Discarding key description data for key ID {}. The \
                            provided buffer is not large enough to contain the data.",
                            id,
                        );
                        continue;
                    }
                    len
                }
            };

            let keyctl_str = str::from_utf8(&keyctl_buffer[..len - 1]).map_err(|e| {
                StratisError::Engine(
                    ErrorEnum::Invalid,
                    format!("Kernel key description was not valid UTF8: {}", e),
                )
            })?;
            let parsed_string = parse_keyctl_describe_string(keyctl_str)?;
            if let Some(kd) = KeyDescription::from_system_key_desc(&parsed_string) {
                key_descs.push(kd.as_application_str().to_string());
            }
        }
        Ok(key_descs)
    }
}

/// Unset the key with ID `key_id` in the root peristent keyring.
fn unset_key(key_id: KeySerial) -> StratisResult<()> {
    let keyring_id = get_persistent_keyring()?;

    match unsafe { syscall(SYS_keyctl, libc::KEYCTL_UNLINK, key_id, keyring_id) } {
        i if i < 0 => Err(io::Error::last_os_error().into()),
        _ => Ok(()),
    }
}

/// Handle for kernel keyring interaction.
#[derive(Debug)]
pub struct StratKeyActions;

#[cfg(test)]
impl StratKeyActions {
    /// Method used in testing to bypass the need to provide a file descriptor
    /// when setting the key. This method allows passing memory to the engine API
    /// for adding keys and removes the need for a backing file or interactive entry
    /// of the key. This method is only useful for testing stratisd internally. It
    /// is not useful for testing using D-Bus.
    pub fn set_no_fd(
        &mut self,
        key_desc: &str,
        key: SizedKeyMemory,
    ) -> StratisResult<MappingCreateAction<()>> {
        Ok(set_key_idem(
            &KeyDescription::from(key_desc.to_string()),
            key,
        )?)
    }
}

impl KeyActions for StratKeyActions {
    fn set(
        &mut self,
        key_desc: &str,
        key_fd: RawFd,
        interactive: bool,
    ) -> StratisResult<MappingCreateAction<()>> {
        let key_file = unsafe { File::from_raw_fd(key_fd) };
        let mut memory = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
        let mut pos = 0;
        let mut bytes_iter = key_file.bytes();
        loop {
            match bytes_iter.next() {
                Some(Ok(b)) => {
                    if interactive && b as char == '\n' {
                        break;
                    }
                    if pos == MAX_STRATIS_PASS_SIZE {
                        if bytes_iter.next().is_some() {
                            return Err(StratisError::Engine(
                                ErrorEnum::Invalid,
                                "Provided key was too long".to_string(),
                            ));
                        }
                        break;
                    }

                    memory.as_mut()[pos] = b;
                    pos += 1;
                }
                Some(Err(e)) => return Err(e.into()),
                None => break,
            }
        }
        let sized_memory = SizedKeyMemory::new(memory, pos);

        Ok(set_key_idem(
            &KeyDescription::from(key_desc.to_string()),
            sized_memory,
        )?)
    }

    fn list(&self) -> StratisResult<Vec<String>> {
        let mut key_ids = KeyIdList::new();
        key_ids.populate()?;
        key_ids.to_key_descs()
    }

    fn read(&self, key_description: &str) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
        read_key(&KeyDescription::from(key_description.to_string())).map(|(opt, _)| opt)
    }

    fn unset(&mut self, key_desc: &str) -> StratisResult<DeleteAction<()>> {
        let keyring_id = get_persistent_keyring()?;

        if let Some(key_id) = search_key(keyring_id, &KeyDescription::from(key_desc.to_string()))? {
            unset_key(key_id).map(|_| DeleteAction::Deleted(()))
        } else {
            Ok(DeleteAction::Identity)
        }
    }
}
