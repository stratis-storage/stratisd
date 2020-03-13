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
    c_uint, CryptActivateFlags, CryptDeactivateFlags, CryptDevice, CryptInit, CryptKeyslot,
    CryptStatusInfo, CryptVolumeKeyFlags, CryptWipePattern, EncryptionFormat, LibcryptErr,
    SafeMemHandle,
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
const STRATIS_KEY_SIZE: usize = 512 / 8;

/// Will be replaced with libc constants in libc v0.2.68
mod consts {
    use libc::c_int;

    pub const KEYCTL_GET_PERSISTENT: c_int = 22;
    pub const KEYCTL_READ: c_int = 11;
    pub const KEY_SPEC_SESSION_KEYRING: c_int = -3;
}

use self::consts::{KEYCTL_GET_PERSISTENT, KEYCTL_READ, KEY_SPEC_SESSION_KEYRING};

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
pub fn is_encrypted_stratis_device(physical_path: &Path) -> Result<bool> {
    if !device_is_luks2(physical_path)? {
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
        return Ok(false);
    }

    Ok(true)
}

/// Returns `true` only if the given device path is for a device encrypted with
/// the LUKS2 format.
pub fn device_is_luks2(physical_path: &Path) -> Result<bool> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    Ok(crypt_device.format_handle().get_type()? == EncryptionFormat::Luks2)
}

/// Read key from keyring with the given key description
///
/// Returns a safe owned memory segment that will clear itself when dropped.
///
/// Requires cryptsetup 2.3
fn read_key(key_description: &str) -> Result<SafeMemHandle> {
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
    let key_description_cstring = CString::new(key_description).map_err(LibcryptErr::NullError)?;

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

/// Activate device by token then check that the logical path exists corresponding
/// to the activation name passed into this method.
fn activate_and_check_device_path(crypt_device: &mut CryptDevice, name: &str) -> Result<PathBuf> {
    // Activate by token
    crypt_device.token_handle().activate_by_token::<()>(
        Some(name),
        Some(LUKS2_TOKEN_ID),
        None,
        CryptActivateFlags::empty(),
    )?;

    // Check activation status.
    if !encrypted_device_is_active(name) {
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

/// Check if an encrypted device's logical devicemapper path
/// is active.
fn encrypted_device_is_active(device_name: &str) -> bool {
    libcryptsetup_rs::status(None, device_name)
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

/// Query the Stratis metadata for the device activation name.
pub fn get_device_name_from_metadata(physical_path: &Path) -> Result<String> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    get_stratis_device_name(&mut crypt_device)
}

/// Deactivate an encrypted Stratis device but do not wipe it. This is not
/// a destructive action. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup.
pub fn deactivate_encrypted_stratis_device(name: &str) -> Result<()> {
    let mut crypt_device = CryptInit::init_by_name_and_header(name, None)?;
    let deactivate_result = crypt_device
        .activate_handle()
        .deactivate(name, CryptDeactivateFlags::empty());
    if deactivate_result.is_err() {
        warn!("Failed to deactivate the crypt device with name {}; you will need to deactivate it manually", name);
    }
    deactivate_result
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

        let max_keyslots = CryptKeyslot::max_keyslots(EncryptionFormat::Luks2)?;
        for i in 0..max_keyslots {
            crypt_device.keyslot_handle().destroy(i)?;
        }

        let (md_size, ks_size) = crypt_device.settings_handle().get_metadata_size()?;
        let total_luks2_metadata_size = *md_size + *ks_size;
        crypt_device.wipe_handle().wipe::<()>(
            physical_path,
            CryptWipePattern::Zero,
            0,
            total_luks2_metadata_size,
            4096,
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
pub fn destroy_encrypted_stratis_device(physical_path: &Path, name: &str) -> Result<()> {
    // Pre-check that name and path are consistent
    let mut crypt_device = CryptInit::init(physical_path)?;
    let metadata_device_name = get_stratis_device_name(&mut crypt_device)?;
    if metadata_device_name.as_str() != name {
        return Err(LibcryptErr::Other(format!(
            "Provided device name and device path are not consistent according to the Stratis \
            metadata. Device name requested in destroy operation was {} while the device name \
            recorded in the Stratis metadata on device {} is {}. Stopping destroy operation.",
            name,
            physical_path.display(),
            metadata_device_name,
        )));
    }
    std::mem::drop(crypt_device);

    if encrypted_device_is_active(name) {
        deactivate_encrypted_stratis_device(name)?;
    }
    wipe_encrypted_stratis_device(physical_path)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        ffi::CString,
        fs::{File, OpenOptions},
        io::{Read, Write},
    };

    use uuid::Uuid;

    use devicemapper::Sectors;

    use crate::engine::strat_engine::tests::{loopbacked, real};

    use super::*;

    fn insert_and_cleanup_key<F>(physical_path: &Path, test: F)
    where
        F: Fn(&Path, &str) -> std::result::Result<(), Box<dyn Error>>,
    {
        let type_cstring = "user\0";
        let description = "test-description-for-stratisd";
        let description_cstring = CString::new(description).unwrap();
        let mut key_data = [0; STRATIS_KEY_SIZE];
        File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut key_data)
            .unwrap();

        // This constant is not in the libc crate yet
        const KEYCTL_UNLINK: i32 = 9;

        let key_id = match unsafe {
            libc::syscall(
                libc::SYS_add_key,
                type_cstring.as_ptr(),
                description_cstring.as_ptr(),
                key_data.as_ptr(),
                key_data.len(),
                KEY_SPEC_SESSION_KEYRING,
            )
        } {
            i if i < 0 => panic!("Failed to create key in keyring"),
            i => i,
        };

        let result = test(physical_path, description);

        if unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_UNLINK,
                key_id,
                KEY_SPEC_SESSION_KEYRING,
            )
        } < 0
        {
            panic!(
                "Failed to clean up key with key description {} from keyring",
                description
            );
        }

        result.unwrap()
    }

    /// Test initializing and activating an encrypted device using
    /// the utilities provided here.
    fn test_crypt_device_ops(paths: &[&Path]) {
        assert_eq!(paths.len(), 1);

        let test = |path: &Path, key_desc: &str| -> std::result::Result<(), Box<dyn Error>> {
            let pool_uuid = Uuid::new_v4();
            let dev_uuid = Uuid::new_v4();

            let logical_path =
                initialize_encrypted_stratis_device(path, pool_uuid, dev_uuid, key_desc)?;

            let mut devicenode = OpenOptions::new().write(true).open(logical_path)?;
            let test_string = "this is a test string to be checked for";
            devicenode.write_all(test_string.as_bytes())?;
            std::mem::drop(devicenode);

            let mut disk_buffer = Vec::new();
            let mut devicenode = File::open(path)?;
            devicenode.read_to_end(&mut disk_buffer)?;
            let lossy_disk_string = String::from_utf8_lossy(&disk_buffer);
            if lossy_disk_string.contains(test_string) {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    "Disk was not encrypted!",
                )));
            }
            std::mem::drop(devicenode);

            let close_active = |name: &str| -> Result<()> {
                let mut crypt_device =
                    libcryptsetup_rs::CryptInit::init_by_name_and_header(name, None)?;
                crypt_device
                    .context_handle()
                    .load::<()>(libcryptsetup_rs::EncryptionFormat::Luks2, None)?;
                crypt_device
                    .activate_handle()
                    .deactivate(name, libcryptsetup_rs::CryptDeactivateFlags::empty())?;

                Ok(())
            };

            let name = name_from_uuids(&pool_uuid, &dev_uuid);
            close_active(&name)?;

            activate_encrypted_stratis_device(path)?;
            close_active(&name)?;

            Ok(())
        };

        insert_and_cleanup_key(paths[0], test);
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
