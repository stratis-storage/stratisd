// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ffi::CString,
    io,
    path::{Path, PathBuf},
};

use libc::{c_void, syscall, SYS_keyctl, SYS_request_key};

use libcryptsetup_rs::{
    c_uint, CryptActivateFlags, CryptDeactivateFlags, CryptDevice, CryptInit, CryptStatusInfo,
    CryptVolumeKeyFlags, CryptWipePattern, EncryptionFormat, LibcryptErr, SafeMemHandle,
};

use crate::engine::{DevUuid, PoolUuid};

type Result<T> = std::result::Result<T, LibcryptErr>;

// Stratis token JSON keys
const TOKEN_TYPE_KEY: &str = "type";
const TOKEN_KEYSLOTS_KEY: &str = "keyslots";
const STRATIS_TOKEN_DEVNAME_KEY: &str = "activation_name";
const STRATIS_TOKEN_KEYDESC_KEY: &str = "key_description";
const STRATIS_TOKEN_POOL_UUID_KEY: &str = "pool_uuid";
const STRATIS_TOKEN_DEV_UUID_KEY: &str = "device_uuid";

const LUKS2_TOKEN_ID: c_uint = 0;
const STRATIS_TOKEN_ID: c_uint = 1;

const LUKS2_TOKEN_TYPE: &str = "luks2-keyring";
const STRATIS_TOKEN_TYPE: &str = "stratis";
#[cfg(not(test))]
const STRATIS_KEY_SIZE: usize = 512 / 8;
#[cfg(test)]
pub const STRATIS_KEY_SIZE: usize = 512 / 8;

/// Sector size as determined in `cryptsetup/lib/internal.h`
const SECTOR_SIZE: u64 = 512;

use self::consts::*;

/// Will be replaced with libc constants in libc v0.2.68
mod consts {
    use libc::c_int;

    pub const KEY_SPEC_SESSION_KEYRING: c_int = -3;
    pub const KEYCTL_READ: c_int = 11;
    pub const KEYCTL_GET_PERSISTENT: c_int = 22;
}

/// Check that the token can open the device.
///
/// No activation will actually occur, only validation.
fn check_luks2_token(crypt_device: &mut CryptDevice) -> Result<()> {
    crypt_device.token_handle().activate_by_token::<()>(
        None,
        Some(LUKS2_TOKEN_ID),
        None,
        CryptActivateFlags::empty(),
    )?;
    Ok(())
}

/// Validate that the LUKS2 token is present and valid
///
/// May not be necessary. See the comment above the invocation.
fn luks2_token_type_is_valid(json: &serde_json::Value) -> bool {
    json.get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == LUKS2_TOKEN_TYPE)
        .unwrap_or(false)
}

/// Get the key description from a LUKS2 keyring token
fn get_key_description(crypt_device: &mut CryptDevice) -> Result<String> {
    let json = crypt_device.token_handle().json_get(LUKS2_TOKEN_ID)?;
    json.get(STRATIS_TOKEN_KEYDESC_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str.to_string())
        .ok_or_else(|| LibcryptErr::Other("Malformed key_description in LUKS2 token".to_string()))
}

/// Get the Stratis activation name from a Stratis token
fn get_stratis_device_name(crypt_device: &mut CryptDevice) -> Result<String> {
    let json = crypt_device.token_handle().json_get(STRATIS_TOKEN_ID)?;
    json.get(STRATIS_TOKEN_DEVNAME_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str.to_string())
        .ok_or_else(|| {
            LibcryptErr::Other("Could not get activation_name from Stratis token".to_string())
        })
}

/// Validate that the Stratis token is present and valid
fn stratis_token_is_valid(json: &serde_json::Value, key_description: String) -> bool {
    json.get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == STRATIS_TOKEN_TYPE)
        .unwrap_or(false)
        && json
            .get(TOKEN_KEYSLOTS_KEY)
            .and_then(|arr| arr.as_array())
            .map(|arr| arr.is_empty())
            .unwrap_or(false)
        && json
            .get(STRATIS_TOKEN_KEYDESC_KEY)
            .and_then(|key| key.as_str())
            .map(|key_str| key_str == key_description.as_str())
            .unwrap_or(false)
        && json
            .get(STRATIS_TOKEN_POOL_UUID_KEY)
            .and_then(|uuid| uuid.as_str())
            .and_then(|uuid_str| uuid::Uuid::from_slice(uuid_str.as_bytes()).ok())
            .is_some()
        && json
            .get(STRATIS_TOKEN_POOL_UUID_KEY)
            .and_then(|uuid| uuid.as_str())
            .and_then(|uuid_str| uuid::Uuid::from_slice(uuid_str.as_bytes()).ok())
            .is_some()
        && json.get(STRATIS_TOKEN_DEVNAME_KEY).is_some()
}

/// Check whether the physical device path corresponds to an encrypted
/// Stratis device.
///
/// This method works on activated and deactivated encrypted devices.
///
/// This device will only return true if the device was initialized
/// with encryption by Stratis. This requires that:
/// * the device is a LUKS2 encrypted device.
/// * the device has a valid Stratis LUKS2 token.
#[allow(dead_code)]
pub fn is_encrypted_stratis_device(physical_path: &Path) -> bool {
    fn device_operations(physical_path: &Path) -> Result<bool> {
        if !device_is_luks2(physical_path) {
            return Ok(false);
        }

        let mut crypt_device = CryptInit::init(physical_path)?;
        crypt_device
            .context_handle()
            .load::<()>(EncryptionFormat::Luks2, None)?;
        check_luks2_token(&mut crypt_device)?;

        // Checking the LUKS2 token type not be entirely necessary as we've already
        // validated that the token can decrypt the volume which should also
        // validate that the appropriate key is in the keyring.
        //
        // Only potential benefit of checking the type of the token is that a LUKS2 token
        // of another type may be present and capable of unlocking a non-Stratis device
        // using something other than the keyring for example.
        //
        // Given that only Stratis devices should have a Stratis token, this may
        // still be superfluous as we will check for that after before determining
        // whether or not it is a Stratis device and nothing is ever activated so
        // we do not need to guard against unlocking a device that is not ours.
        let luks_json = crypt_device.token_handle().json_get(LUKS2_TOKEN_ID)?;
        let key_description = get_key_description(&mut crypt_device)?;
        let stratis_json = crypt_device.token_handle().json_get(STRATIS_TOKEN_ID)?;
        if !luks2_token_type_is_valid(&luks_json)
            || !stratis_token_is_valid(&stratis_json, key_description)
        {
            Ok(false)
        } else {
            Ok(true)
        }
    }

    match device_operations(physical_path) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                "Operations querying device {} to determine if it is a Stratis device \
                failed with an error: {}; reporting as not a Stratis device",
                physical_path.display(),
                e
            );
            false
        }
    }
}

/// Returns `true` only if the given device path is for a device encrypted with
/// the LUKS2 format.
pub fn device_is_luks2(physical_path: &Path) -> bool {
    let mut crypt_device = match CryptInit::init(physical_path) {
        Ok(d) => d,
        Err(e) => {
            warn!(
                "Getting devicemapper context for device {} \
                failed with error: {}; reporting as not a LUKS2 device.",
                physical_path.display(),
                e
            );
            return false;
        }
    };
    crypt_device.format_handle().get_type().ok() == Some(EncryptionFormat::Luks2)
}

/// Read key from keyring with the given key description
///
/// Returns a safe owned memory segment that will clear itself when dropped.
///
/// Requires cryptsetup 2.3
fn read_key(key_description: &str) -> Result<SafeMemHandle> {
    fn read_key_result(key_description: &str) -> Result<SafeMemHandle> {
        // Attach persistent keyring to process keyring
        match unsafe {
            syscall(
                SYS_keyctl,
                KEYCTL_GET_PERSISTENT,
                0,
                KEY_SPEC_SESSION_KEYRING,
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            _ => (),
        };

        let key_type_cstring = CString::new("user").expect("String is valid");
        let key_description_cstring =
            CString::new(key_description).map_err(LibcryptErr::NullError)?;

        // Request key ID from persistent keyring
        let key_id = match unsafe {
            syscall(
                SYS_request_key,
                key_type_cstring.as_ptr(),
                key_description_cstring.as_ptr(),
                std::ptr::null::<c_void>(),
                0,
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            i => i,
        };

        let mut key_buffer = SafeMemHandle::alloc(STRATIS_KEY_SIZE)?;
        let mut_ref = key_buffer.as_mut();

        // Read key from keyring
        match unsafe {
            syscall(
                SYS_keyctl,
                KEYCTL_READ,
                key_id,
                mut_ref.as_mut_ptr(),
                mut_ref.len(),
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            _ => (),
        };
        Ok(key_buffer)
    }

    let read_key_result = read_key_result(key_description);
    if read_key_result.is_err() {
        warn!(
            "Failed to read the key with key description {} from the keyring; \
            encryption cannot continue",
            key_description
        );
    }
    read_key_result
}

/// Activate device by token then check that the logical path exists corresponding
/// to the activation name passed into this method.
fn activate_and_check_device_path(crypt_device: &mut CryptDevice, name: &str) -> Result<PathBuf> {
    fn activation_result(crypt_device: &mut CryptDevice, name: &str) -> Result<PathBuf> {
        // Activate by token
        crypt_device.token_handle().activate_by_token::<()>(
            Some(name),
            Some(LUKS2_TOKEN_ID),
            None,
            CryptActivateFlags::empty(),
        )?;

        // Check activation status.
        if !device_is_active(crypt_device, name) {
            return Err(LibcryptErr::Other("Failed to activate device".to_string()));
        }

        // Checking that the symlink was created may also be valuable in case a race
        // condition occurs with udev.
        let mut activated_path = PathBuf::from("/dev/mapper");
        activated_path.push(name);

        // Can potentially use inotify with a timeout to wait for the symlink
        // if race conditions become a problem.
        if activated_path.exists() {
            Ok(activated_path)
        } else {
            Err(LibcryptErr::IOError(io::Error::from(
                io::ErrorKind::NotFound,
            )))
        }
    }

    let activation_result = activation_result(crypt_device, name);
    if activation_result.is_err() {
        warn!("Failed to activate device with name {}", name);
    }
    activation_result
}

/// Check if the given device is active.
fn encrypted_device_is_active(physical_path: &Path, device_name: &str) -> bool {
    fn is_active(physical_path: &Path, device_name: &str) -> Result<bool> {
        let mut device = CryptInit::init(physical_path)?;
        device
            .context_handle()
            .load::<()>(EncryptionFormat::Luks2, None)?;
        Ok(device_is_active(&mut device, device_name))
    }

    let active_result = is_active(physical_path, device_name);
    match active_result {
        Ok(b) => b,
        Err(e) => {
            warn!(
                "Encountered an error checking activation status of \
                device {}: {}; reporting as inactive",
                physical_path.display(),
                e
            );
            false
        }
    }
}

fn device_is_active(device: &mut CryptDevice, device_name: &str) -> bool {
    libcryptsetup_rs::status(Some(device), device_name)
        .map(|status| status == CryptStatusInfo::Active)
        .unwrap_or(false)
}

/// Get a devicemapper name from the pool and device UUIDs.
pub fn name_from_uuids(pool_uuid: &PoolUuid, dev_uuid: &DevUuid) -> String {
    format!("{}-{}", pool_uuid.to_simple_ref(), dev_uuid.to_simple_ref())
}

/// Lay down properly configured LUKS2 metadata on a new physical device
pub fn initialize_encrypted_stratis_device(
    physical_path: &Path,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
    key_description: &str,
) -> Result<PathBuf> {
    let mut crypt_device = CryptInit::init(physical_path)?;

    crypt_device.context_handle().format::<()>(
        EncryptionFormat::Luks2,
        ("aes", "xts-plain64"),
        None,
        libcryptsetup_rs::Either::Right(STRATIS_KEY_SIZE),
        None,
    )?;
    let key = read_key(key_description)?;

    let keyslot = crypt_device.keyslot_handle().add_by_key(
        None,
        None,
        key.as_ref(),
        CryptVolumeKeyFlags::empty(),
    )?;

    // Initialize keyring token
    crypt_device
        .token_handle()
        .luks2_keyring_set(Some(LUKS2_TOKEN_ID), key_description)?;
    crypt_device
        .token_handle()
        .assign_keyslot(LUKS2_TOKEN_ID, Some(keyslot))?;

    // The default activation name is [POOLUUID]-[DEVUUID] which should be unique
    // across all Stratis pools.
    let activation_name = name_from_uuids(&pool_uuid, &dev_uuid);

    // Initialize stratis token
    crypt_device.token_handle().json_set(
        Some(STRATIS_TOKEN_ID),
        &json!({
            TOKEN_TYPE_KEY: STRATIS_TOKEN_TYPE,
            TOKEN_KEYSLOTS_KEY: [],
            STRATIS_TOKEN_POOL_UUID_KEY: pool_uuid.to_simple_ref().to_string(),
            STRATIS_TOKEN_DEV_UUID_KEY: dev_uuid.to_simple_ref().to_string(),
            STRATIS_TOKEN_KEYDESC_KEY: key_description,
            STRATIS_TOKEN_DEVNAME_KEY: activation_name,
        }),
    )?;

    activate_and_check_device_path(&mut crypt_device, &activation_name)
}

/// Activate encrypted Stratis device using the name stored in the
/// Stratis token
#[allow(dead_code)]
pub fn activate_encrypted_stratis_device(physical_path: &Path) -> Result<PathBuf> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    crypt_device
        .context_handle()
        .load::<()>(EncryptionFormat::Luks2, None)?;

    let stratis_device_name = get_stratis_device_name(&mut crypt_device)?;

    activate_and_check_device_path(&mut crypt_device, &stratis_device_name)
}

/// Get the path of the physical device backing the encrypted volume
pub fn get_physical_device_path_from_name(name: &str) -> Result<PathBuf> {
    let mut crypt_device = CryptInit::init_by_name_and_header(name, None)?;
    Ok(crypt_device.status_handle().get_device_path()?.to_owned())
}

/// Query the Stratis metadata for the device activation name.
fn get_device_name_from_metadata(physical_path: &Path) -> Result<String> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    crypt_device
        .context_handle()
        .load::<()>(EncryptionFormat::Luks2, None)?;
    get_stratis_device_name(&mut crypt_device)
}

/// Get a list of all keyslots associated with the LUKS2 token.
/// This is necessary because attempting to destroy an uninitialized
/// keyslot will result in an error.
fn get_keyslot_number(device: &mut CryptDevice) -> Result<Vec<c_uint>> {
    let json = device.token_handle().json_get(LUKS2_TOKEN_ID)?;
    let vec = json
        .get(TOKEN_KEYSLOTS_KEY)
        .and_then(|k| k.as_array())
        .ok_or_else(|| LibcryptErr::Other("keyslots value was malformed".to_string()))?;
    Ok(vec
        .iter()
        .filter_map(|int_val| {
            let as_str = int_val.as_str();
            if as_str.is_none() {
                warn!(
                    "Discarding invalid value in LUKS2 token keyslot array: {}",
                    int_val
                );
            }
            let s = match as_str {
                Some(s) => s,
                None => return None,
            };
            let as_u64 = s.parse::<u64>();
            if let Err(ref e) = as_u64 {
                warn!(
                    "Discard invalid value in LUKS2 token keyslot array: {}; \
                    failed to convert it to an integer: {}",
                    s, e,
                );
            }
            as_u64.ok()
        })
        .map(|int| int as c_uint)
        .collect::<Vec<_>>())
}

/// Deactivate an encrypted Stratis device but do not wipe it. This is not
/// a destructive action. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup.
pub fn deactivate_encrypted_stratis_device(name: &str) -> Result<()> {
    fn deactivate_device(name: &str) -> Result<()> {
        let mut crypt_device = CryptInit::init_by_name_and_header(name, None)?;
        crypt_device
            .context_handle()
            .load::<()>(EncryptionFormat::Luks2, None)?;
        crypt_device
            .activate_handle()
            .deactivate(name, CryptDeactivateFlags::empty())?;
        Ok(())
    }

    let deactivate_result = deactivate_device(name);
    if deactivate_result.is_err() {
        warn!("Failed to deactivate the crypt device with name {}; you will need to deactivate it manually", name);
    }
    deactivate_result
}

/// Align the number of bytes to the nearest multiple of `SECTOR_SIZE`
/// above the current value.
pub fn ceiling_sector_size_alignment(bytes: u64) -> u64 {
    bytes + (SECTOR_SIZE - (bytes % SECTOR_SIZE))
}

/// Destroy all keyslots and wipe the LUKS2 metadata from a physical volume.
/// This should only be used if the device has already been deactivated.
/// Otherwise, there will be a hanging devicemapper device left on the system.
/// To destroy an active volume, use `destroy_encrypted_stratis_device`.
fn wipe_encrypted_stratis_device(physical_path: &Path) -> Result<()> {
    fn destroy_slots_and_wipe(physical_path: &Path) -> Result<()> {
        let mut crypt_device = CryptInit::init(physical_path)?;
        crypt_device
            .context_handle()
            .load::<()>(EncryptionFormat::Luks2, None)?;

        for i in get_keyslot_number(&mut crypt_device)?.iter() {
            crypt_device.keyslot_handle().destroy(*i)?;
        }

        let (md_size, ks_size) = crypt_device.settings_handle().get_metadata_size()?;
        let total_luks2_metadata_size = *md_size + *ks_size;
        crypt_device.wipe_handle().wipe::<()>(
            physical_path,
            CryptWipePattern::Zero,
            0,
            ceiling_sector_size_alignment(total_luks2_metadata_size),
            SECTOR_SIZE as usize,
            false,
            None,
            None,
        )
    }

    let wipe_result = destroy_slots_and_wipe(physical_path);
    if wipe_result.is_err() {
        warn!(
            "Failed to wipe LUKS2 metadata from device {}; you will need to clean it up manually",
            physical_path.display()
        );
    }
    wipe_result
}

/// Deactivate and wipe the encrypted device. This is a destructive action and data
/// will not be able to be recovered. Both physical path of the device to be wiped
/// and the name of the activated device must be provided.
pub fn destroy_encrypted_stratis_device(physical_path: &Path) -> Result<()> {
    let metadata_device_name = get_device_name_from_metadata(physical_path);
    if metadata_device_name.is_err() {
        warn!("Failed to get active device name from Stratis LUKS2 metadata; cannot clean up encrypted volume");
    }
    let name = metadata_device_name?;

    if encrypted_device_is_active(physical_path, &name) {
        deactivate_encrypted_stratis_device(&name)?;
    }
    wipe_encrypted_stratis_device(physical_path)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{File, OpenOptions},
        io::{Read, Write},
    };

    use uuid::Uuid;

    use devicemapper::Sectors;

    use crate::{
        engine::strat_engine::tests::{crypt, loopbacked, real},
        stratis::StratisError,
    };

    use super::*;

    /// Test initializing and activating an encrypted device using
    /// the utilities provided here.
    fn test_crypt_device_ops(paths: &[&Path]) {
        fn crypt_test(paths: &[&Path], key_desc: &str) -> std::result::Result<(), Box<dyn Error>> {
            let path = paths.get(0).ok_or_else(|| {
                Box::new(StratisError::Error(
                    "This test only accepts a single device".to_string(),
                ))
            })?;

            let pool_uuid = Uuid::new_v4();
            let dev_uuid = Uuid::new_v4();

            let logical_path =
                initialize_encrypted_stratis_device(path, pool_uuid, dev_uuid, key_desc)?;

            let mut devicenode = OpenOptions::new().write(true).open(logical_path)?;
            let mut random_buffer = [0; 32];
            File::open("/dev/urandom")?.read_exact(&mut random_buffer)?;
            devicenode.write_all(&random_buffer)?;
            std::mem::drop(devicenode);

            let mut disk_buffer = Vec::new();
            let mut devicenode = File::open(path)?;
            devicenode.read_to_end(&mut disk_buffer)?;
            for window in disk_buffer.windows(32) {
                if window == random_buffer {
                    return Err(Box::new(io::Error::new(
                        io::ErrorKind::Other,
                        "Disk was not encrypted!",
                    )));
                }
            }
            std::mem::drop(devicenode);

            let name = name_from_uuids(&pool_uuid, &dev_uuid);
            deactivate_encrypted_stratis_device(&name)?;

            activate_encrypted_stratis_device(path)?;
            destroy_encrypted_stratis_device(path)?;

            Ok(())
        }

        assert_eq!(paths.len(), 1);

        crypt::insert_and_cleanup_key(paths, crypt_test);
    }

    #[test]
    fn loop_test_crypt_device_ops() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn real_test_crypt_device_ops() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn travis_test_crypt_device_ops() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_crypt_device_ops,
        );
    }
}
