// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{ffi::CString, io, mem::size_of, os::unix::io::RawFd, str};

use libc::{syscall, SYS_add_key, SYS_keyctl};

use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        shared,
        strat_engine::names::KeyDescription,
        types::{Key, MappingCreateAction, MappingDeleteAction, SizedKeyMemory},
    },
    stratis::{StratisError, StratisResult},
};

/// A type corresponding to key IDs in the kernel keyring. In `libkeyutils`,
/// this is represented as the C type `key_serial_t`.
type KeySerial = u32;

/// Search the persistent keyring for the given key description.
pub(super) fn search_key_persistent(key_desc: &KeyDescription) -> StratisResult<Option<KeySerial>> {
    let keyring_id = get_persistent_keyring()?;
    search_key(keyring_id, key_desc)
}

/// Read a key from the persistent keyring with the given key description.
pub(super) fn read_key_persistent(
    key_desc: &KeyDescription,
) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
    let keyring_id = get_persistent_keyring()?;
    read_key(keyring_id, key_desc)
}

/// Get the ID of the persistent root user keyring and attach it to
/// the session keyring.
pub fn get_persistent_keyring() -> StratisResult<KeySerial> {
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
        i => convert_int!(i, libc::c_long, KeySerial),
    }
}

/// Search for the given key description in the persistent root keyring.
/// Returns the key ID or nothing if it was not found in the keyring.
fn search_key(
    keyring_id: KeySerial,
    key_desc: &KeyDescription,
) -> StratisResult<Option<KeySerial>> {
    let key_desc_cstring = CString::new(key_desc.to_system_string())
        .map_err(|_| StratisError::Msg("Invalid key description provided".to_string()))?;

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
        convert_int!(key_id, libc::c_long, KeySerial).map(Some)
    }
}

/// Read a key with the provided key description into safely handled memory if it
/// exists in the keyring.
///
/// The return type will be a tuple of an `Option` and a keyring id. The `Option`
/// type will be `Some` if the key was found in the keyring and will contain
/// the key ID and the key contents. If no key was found with the provided
/// key description, `None` will be returned.
fn read_key(
    keyring_id: KeySerial,
    key_desc: &KeyDescription,
) -> StratisResult<Option<(KeySerial, SizedKeyMemory)>> {
    let key_id_option = search_key(keyring_id, key_desc)?;
    let key_id = if let Some(ki) = key_id_option {
        ki
    } else {
        return Ok(None);
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
        i => Ok(Some((
            key_id as KeySerial,
            SizedKeyMemory::new(key_buffer, convert_int!(i, libc::c_long, usize)?),
        ))),
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
    let key_desc_cstring = CString::new(key_desc.to_system_string())
        .map_err(|_| StratisError::Msg("Invalid key description provided".to_string()))?;
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
/// The unit type is returned as the inner type for `MappingCreateAction` as no
/// new external data (like a UUID) can be returned when setting a key. Keys
/// are identified by their key descriptions only unlike resources like pools
/// that have a name and a UUID.
///
/// Successful return values:
/// * `Ok(MappingCreateAction::Identity)`: The key was already in the keyring with the
///   appropriate key description and key data.
/// * `Ok(MappingCreateAction::Created(()))`: The key was newly added to the keyring.
/// * `Ok(MappingCreateAction::ValueChanged(()))`: The key description was already present
///   in the keyring but the key data was updated.
fn set_key_idem(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
) -> StratisResult<MappingCreateAction<Key>> {
    let keyring_id = get_persistent_keyring()?;
    match read_key(keyring_id, key_desc) {
        Ok(Some((key_id, old_key_data))) => {
            let changed = reset_key(key_id, old_key_data, key_data)?;
            if changed {
                Ok(MappingCreateAction::ValueChanged(Key))
            } else {
                Ok(MappingCreateAction::Identity)
            }
        }
        Ok(None) => {
            set_key(key_desc, key_data, keyring_id)?;
            Ok(MappingCreateAction::Created(Key))
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
            StratisError::Msg(
                "Invalid format returned from the kernel query for the key description".to_string(),
            )
        })
}

/// A list of key IDs that were read from the persistent root keyring.
struct KeyIdList {
    key_ids: Vec<KeySerial>,
}

impl KeyIdList {
    /// Create a new list of key IDs, with initial capacity of 4096
    fn new() -> KeyIdList {
        KeyIdList {
            key_ids: Vec::with_capacity(4096),
        }
    }

    /// Populate the list with IDs from the persistent root kernel keyring.
    fn populate(&mut self) -> StratisResult<()> {
        let keyring_id = get_persistent_keyring()?;

        // Read list of keys in the persistent keyring.
        let mut done = false;
        while !done {
            let num_bytes_read = match unsafe {
                syscall(
                    SYS_keyctl,
                    libc::KEYCTL_READ,
                    keyring_id,
                    self.key_ids.as_mut_ptr(),
                    self.key_ids.capacity(),
                )
            } {
                i if i < 0 => return Err(io::Error::last_os_error().into()),
                i => convert_int!(i, libc::c_long, usize)?,
            };

            let num_key_ids = num_bytes_read / size_of::<KeySerial>();

            if num_key_ids <= self.key_ids.capacity() {
                unsafe {
                    self.key_ids.set_len(num_key_ids);
                }
                done = true;
            } else {
                self.key_ids.resize(num_key_ids, 0);
            }
        }

        Ok(())
    }

    /// Get the list of key descriptions corresponding to the kernel key IDs.
    /// Return the subset of key descriptions that have a prefix that identify
    /// them as belonging to Stratis.
    fn to_key_descs(&self) -> StratisResult<Vec<KeyDescription>> {
        let mut key_descs = Vec::new();

        for id in self.key_ids.iter() {
            let mut keyctl_buffer: Vec<u8> = Vec::with_capacity(4096);

            let mut done = false;
            while !done {
                let len = match unsafe {
                    syscall(
                        SYS_keyctl,
                        libc::KEYCTL_DESCRIBE,
                        *id,
                        keyctl_buffer.as_mut_ptr(),
                        keyctl_buffer.capacity(),
                    )
                } {
                    i if i < 0 => return Err(io::Error::last_os_error().into()),
                    i => convert_int!(i, libc::c_long, usize)?,
                };

                if len <= keyctl_buffer.capacity() {
                    unsafe {
                        keyctl_buffer.set_len(len);
                    }
                    done = true;
                } else {
                    keyctl_buffer.resize(len, 0);
                }
            }

            if keyctl_buffer.is_empty() {
                return Err(StratisError::Msg(format!(
                    "Kernel key description for key {id} appeared to be entirely empty"
                )));
            }

            let keyctl_str =
                str::from_utf8(&keyctl_buffer[..keyctl_buffer.len() - 1]).map_err(|e| {
                    StratisError::Chained(
                        "Kernel key description was not valid UTF8".to_string(),
                        Box::new(StratisError::from(e)),
                    )
                })?;
            let parsed_string = parse_keyctl_describe_string(keyctl_str)?;
            if let Some(kd) = KeyDescription::from_system_key_desc(&parsed_string).map(|k| k.expect("parse_keyctl_describe_string() ensures the key description can not have semi-colons in it")) {
                key_descs.push(kd);
            }
        }
        Ok(key_descs)
    }
}

/// Unset the key with ID `key_id` in the root persistent keyring.
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
        key_desc: &KeyDescription,
        key: SizedKeyMemory,
    ) -> StratisResult<MappingCreateAction<Key>> {
        set_key_idem(key_desc, key)
    }
}

impl KeyActions for StratKeyActions {
    fn set(
        &self,
        key_desc: &KeyDescription,
        key_fd: RawFd,
    ) -> StratisResult<MappingCreateAction<Key>> {
        let mut memory = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
        let bytes = shared::set_key_shared(key_fd, memory.as_mut())?;

        set_key_idem(key_desc, SizedKeyMemory::new(memory, bytes))
    }

    fn list(&self) -> StratisResult<Vec<KeyDescription>> {
        let mut key_ids = KeyIdList::new();
        key_ids.populate()?;
        key_ids.to_key_descs()
    }

    fn unset(&self, key_desc: &KeyDescription) -> StratisResult<MappingDeleteAction<Key>> {
        let keyring_id = get_persistent_keyring()?;

        if let Some(key_id) = search_key(keyring_id, key_desc)? {
            unset_key(key_id).map(|_| MappingDeleteAction::Deleted(Key))
        } else {
            Ok(MappingDeleteAction::Identity)
        }
    }
}
