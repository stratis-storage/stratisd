// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ffi::CString,
    fs::File,
    io::{self, Read},
    os::unix::io::{FromRawFd, RawFd},
};

use libc::{syscall, SYS_add_key, SYS_keyctl};
use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
        types::{CreateAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Read a key with the provided key description into safely handled memory if it
/// exists in the keyring.
///
/// The return type with be a tuple of an `Option` and a keyring id. The `Option`
/// type will be `Some` if the key was found in the keyring and will contain
/// the key ID and the key contents. If no key was found with the provided
/// key description, `None` will be returned.
pub fn read_key(key_desc: &str) -> StratisResult<(Option<(u64, SizedKeyMemory)>, u64)> {
    // Attach persistent keyring to process keyring
    let persistent_id = match unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_GET_PERSISTENT,
            0,
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    } {
        i if i < 0 => return Err(io::Error::last_os_error().into()),
        i => i,
    };

    let key_desc_cstring = CString::new(key_desc).map_err(|_| {
        StratisError::Engine(
            ErrorEnum::Invalid,
            "Invalid key description provided".to_string(),
        )
    })?;

    let key_id = unsafe {
        syscall(
            SYS_keyctl,
            libc::KEYCTL_SEARCH,
            persistent_id,
            concat!("user", "\0").as_ptr(),
            key_desc_cstring.as_ptr(),
        )
    };
    if key_id < 0 {
        if unsafe { *libc::__errno_location() } == libc::ENOKEY {
            return Ok((None, persistent_id as u64));
        } else {
            return Err(io::Error::last_os_error().into());
        }
    }

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
            Some((key_id as u64, SizedKeyMemory::new(key_buffer, i as usize))),
            persistent_id as u64,
        )),
    }
}

/// Update the key attached to the provided key description if the new key data
/// is different from the old key data.
// Precondition: The key description is already present in the keyring.
fn update_key(
    key_id: u64,
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

/// Add the key to the given keyring under with the provided key description.
// Precondition: The key description was not already present.
fn add_key(key_desc: &str, key_data: SizedKeyMemory, keyring_id: u64) -> StratisResult<()> {
    let key_desc_cstring = CString::new(key_desc).map_err(|_| {
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
/// * `Ok(CreateAction::Identity)`: The key was already in the keyring with the
/// appropriate key description and key data.
/// * `Ok(CreateAction::Created(false)`: The key was newly added to the keyring.
/// * `Ok(CreateAction::Created(true)`: The key description was already present
/// in the keyring but the key data was updated.
fn add_key_idem(key_desc: &str, key_data: SizedKeyMemory) -> StratisResult<CreateAction<bool>> {
    match read_key(key_desc) {
        Ok((Some((key_id, old_key_data)), _)) => {
            let changed = update_key(key_id, old_key_data, key_data)?;
            if changed {
                Ok(CreateAction::Created(true))
            } else {
                Ok(CreateAction::Identity)
            }
        }
        Ok((None, keyring_id)) => {
            add_key(key_desc, key_data, keyring_id)?;
            Ok(CreateAction::Created(false))
        }
        Err(e) => Err(e),
    }
}

/// Handle for kernel keyring interaction.
#[derive(Debug)]
pub struct StratKeyActions;

impl KeyActions for StratKeyActions {
    fn add(&mut self, key_desc: &str, key_fd: RawFd) -> StratisResult<CreateAction<bool>> {
        let key_file = unsafe { File::from_raw_fd(key_fd) };
        let mut memory = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
        let mut pos = 0;
        for byte in key_file.bytes() {
            match byte.map(|b| b as char) {
                Ok('\n') => break,
                Ok(c) => {
                    if pos >= MAX_STRATIS_PASS_SIZE {
                        break;
                    }

                    memory.as_mut()[pos] = c as u8;
                    pos += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let sized_memory = SizedKeyMemory::new(memory, pos);

        Ok(add_key_idem(key_desc, sized_memory)?)
    }

    fn read(&self, key_description: &str) -> StratisResult<Option<(u64, SizedKeyMemory)>> {
        read_key(key_description).map(|(opt, _)| opt)
    }
}
