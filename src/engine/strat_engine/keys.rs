// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    convert::TryFrom,
    ffi::CString,
    fs::File,
    io::{self, Read},
    iter::Take,
    os::unix::io::{FromRawFd, RawFd},
    slice::Iter,
    str,
};

use libc::{syscall, SYS_add_key, SYS_keyctl};

use devicemapper::Bytes;
use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        strat_engine::names::KeyDescription,
        types::{DeleteAction, KeySerial, MappingCreateAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

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
/// The unit type is returned as the inner type for `MappingCreateAction` as no
/// new external data (like a UUID) can be returned when setting a key. Keys
/// are identified by their key descriptions only unlike resources like pools
/// that have a name and a UUID.
///
/// Successful return values:
/// * `Ok(MappingCreateAction::Identity)`: The key was already in the keyring with the
/// appropriate key description and key data.
/// * `Ok(MappingCreateAction::Created(()))`: The key was newly added to the keyring.
/// * `Ok(MappingCreateAction::ValueChanged(()))`: The key description was already present
/// in the keyring but the key data was updated.
fn set_key_idem(
    key_desc: &KeyDescription,
    key_data: SizedKeyMemory,
) -> StratisResult<MappingCreateAction<()>> {
    match read_key(key_desc) {
        Ok((Some((key_id, old_key_data)), _)) => {
            let changed = reset_key(key_id, old_key_data, key_data)?;
            if changed {
                Ok(MappingCreateAction::ValueChanged(()))
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
/// This list must keep track of the size externally to the Vec, because the
/// elements in the Vec are allocated by means of a syscall which fills the
/// Vec's internal buffer, rather than by Vec operations, so key_ids.len()
/// will always be 0.
struct KeyIdList {
    key_ids: Vec<KeySerial>,
    num_key_ids: usize,
}

impl KeyIdList {
    /// Create a new list of key IDs, with initial capacity of 4096
    fn new() -> KeyIdList {
        KeyIdList {
            key_ids: Vec::with_capacity(4096),
            num_key_ids: 0,
        }
    }

    /// Populate the list with IDs from the persistent root kernel keyring.
    fn populate(&mut self) -> StratisResult<()> {
        let keyring_id = get_persistent_keyring()?;

        // Read list of keys in the persistent keyring.
        let mut done = false;
        while !done {
            let num_key_ids = match unsafe {
                syscall(
                    SYS_keyctl,
                    libc::KEYCTL_READ,
                    keyring_id,
                    self.key_ids.as_mut_ptr(),
                    self.key_ids.capacity(),
                )
            } {
                i if i < 0 => return Err(io::Error::last_os_error().into()),
                i => i as usize,
            };

            if num_key_ids > self.key_ids.capacity() {
                self.key_ids.reserve(num_key_ids - self.key_ids.capacity());
            } else {
                self.num_key_ids = num_key_ids;
                done = true;
            }
        }

        Ok(())
    }

    /// Get the number of key IDs currently stored in this list.
    fn len(&self) -> usize {
        self.num_key_ids
    }

    /// Iterate through the key IDs.
    fn iter(&self) -> Take<Iter<KeySerial>> {
        self.key_ids.iter().take(self.len())
    }

    /// Get the list of key descriptions corresponding to the kernel key IDs.
    /// Return the subset of key descriptions that have a prefix that identify
    /// them as belonging to Stratis.
    fn to_key_descs(&self) -> StratisResult<Vec<String>> {
        let mut key_descs = Vec::new();

        for id in self.iter() {
            let mut keyctl_buffer: Vec<u8> = Vec::with_capacity(4096);

            let mut description_length = None;
            while description_length.is_none() {
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
                    i => i as usize,
                };

                if len > keyctl_buffer.capacity() {
                    keyctl_buffer.reserve(len - keyctl_buffer.capacity());
                } else {
                    description_length = Some(len);
                }
            }

            let description_length = description_length.expect("must be Some to exit loop");

            if description_length == 0 {
                return Err(StratisError::Error(format!(
                    "Kernel key description for key {} appeared to be entirely empty",
                    id
                )));
            }

            let keyctl_str =
                str::from_utf8(&keyctl_buffer[..description_length - 1]).map_err(|e| {
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
            &KeyDescription::try_from(key_desc.to_string())?,
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
        let mut bytes_iter = key_file.bytes();

        let mut pos = 0;
        while pos < MAX_STRATIS_PASS_SIZE {
            match bytes_iter.next() {
                Some(Ok(b)) => {
                    if interactive && b as char == '\n' {
                        break;
                    }

                    memory.as_mut()[pos] = b;
                    pos += 1;
                }
                Some(Err(e)) => return Err(e.into()),
                None => break,
            }
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

        let sized_memory = SizedKeyMemory::new(memory, pos);

        Ok(set_key_idem(
            &KeyDescription::try_from(key_desc.to_string())?,
            sized_memory,
        )?)
    }

    fn list(&self) -> StratisResult<Vec<String>> {
        let mut key_ids = KeyIdList::new();
        key_ids.populate()?;
        key_ids.to_key_descs()
    }

    fn unset(&mut self, key_desc: &str) -> StratisResult<DeleteAction<()>> {
        let keyring_id = get_persistent_keyring()?;

        if let Some(key_id) =
            search_key(keyring_id, &KeyDescription::try_from(key_desc.to_string())?)?
        {
            unset_key(key_id).map(|_| DeleteAction::Deleted(()))
        } else {
            Ok(DeleteAction::Identity)
        }
    }
}
