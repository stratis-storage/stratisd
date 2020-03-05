#![allow(dead_code)]

use std::{
    ffi::CString,
    io,
    path::{Path, PathBuf},
};

use libc::{c_void, syscall, SYS_keyctl, SYS_request_key};

use libcryptsetup_rs::{
    c_uint, CryptActivateFlags, CryptDevice, CryptInit, CryptStatusInfo, CryptVolumeKeyFlags,
    EncryptionFormat, LibcryptErr, SafeMemHandle,
};

use crate::engine::{DevUuid, PoolUuid};

type Result<T> = std::result::Result<T, LibcryptErr>;

pub static LUKS2_TOKEN_ID: c_uint = 0;
pub static STRATIS_TOKEN_ID: c_uint = 1;
pub static STRATIS_TOKEN_TYPE: &str = "stratis";
pub static STRATIS_KEY_SIZE: usize = 512 / 8;

mod consts {
    use libc::c_int;

    pub const KEYCTL_GET_PERSISTENT: c_int = 22;
    pub const KEYCTL_READ: c_int = 11;
    pub const KEY_SPEC_PROCESS_KEYRING: c_int = -2;
}

use self::consts::{KEYCTL_GET_PERSISTENT, KEYCTL_READ, KEY_SPEC_PROCESS_KEYRING};

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
    json.get("type")
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == "luks2-keyring")
        .unwrap_or(false)
}

/// Get the key description from a LUKS2 keyring token
fn get_key_description(crypt_device: &mut CryptDevice) -> Result<String> {
    let json = crypt_device.token_handle().json_get(LUKS2_TOKEN_ID)?;
    json.get("key_description")
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str.to_string())
        .ok_or_else(|| LibcryptErr::Other("Malformed key_description in LUKS2 token".to_string()))
}

/// Get the Stratis activation name from a Stratis token
fn get_stratis_device_name(crypt_device: &mut CryptDevice) -> Result<String> {
    let json = crypt_device.token_handle().json_get(STRATIS_TOKEN_ID)?;
    json.get("activation_name")
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str.to_string())
        .ok_or_else(|| {
            LibcryptErr::Other("Could not get activation_name from Stratis token".to_string())
        })
}

/// Validate that the Stratis token is present and valid
fn stratis_token_is_valid(json: &serde_json::Value, key_description: String) -> bool {
    json.get("type")
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == STRATIS_TOKEN_TYPE)
        .unwrap_or(false)
        && json
            .get("keyslots")
            .and_then(|arr| arr.as_array())
            .map(|arr| arr.is_empty())
            .unwrap_or(false)
        && json
            .get("key_description")
            .and_then(|key| key.as_str())
            .map(|key_str| key_str == key_description.as_str())
            .unwrap_or(false)
        && json
            .get("pool_uuid")
            .and_then(|uuid| uuid.as_str())
            .and_then(|uuid_str| uuid::Uuid::from_slice(uuid_str.as_bytes()).ok())
            .is_some()
        && json
            .get("device_uuid")
            .and_then(|uuid| uuid.as_str())
            .and_then(|uuid_str| uuid::Uuid::from_slice(uuid_str.as_bytes()).ok())
            .is_some()
        && json.get("activation_name").is_some()
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

// Read key from keyring with the given key description
//
// Returns a safe owned memory segment that will clear itself when dropped.
//
// Requires cryptsetup 2.3
fn read_key(key_description: &str) -> Result<SafeMemHandle> {
    // Attach persistent keyring to process keyring
    match unsafe {
        syscall(
            SYS_keyctl,
            KEYCTL_GET_PERSISTENT,
            0,
            KEY_SPEC_PROCESS_KEYRING,
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

// Activate device by token then check that the logical path exists corresponding
// to the activation name passed into this method.
fn activate_and_check_device_path(crypt_device: &mut CryptDevice, name: &str) -> Result<PathBuf> {
    // Activate by token
    crypt_device.token_handle().activate_by_token::<()>(
        Some(name),
        Some(LUKS2_TOKEN_ID),
        None,
        CryptActivateFlags::empty(),
    )?;

    // Check activation status.
    let status = crypt_device.status_handle().status(name)?;
    if CryptStatusInfo::Active != status {
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

    let keyslot = crypt_device.keyslot_handle(None).add_by_key(
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

    let activation_name = format!("{}-{}", pool_uuid.to_simple_ref(), dev_uuid.to_simple_ref());

    // Initialize stratis token
    crypt_device.token_handle().json_set(
        Some(STRATIS_TOKEN_ID),
        &json!({
            "type": STRATIS_TOKEN_TYPE,
            "keyslots": [],
            "pool_uuid": pool_uuid.to_simple_ref().to_string(),
            "device_uuid": dev_uuid.to_simple_ref().to_string(),
            "key_description": key_description,
            "activation_name": activation_name,
        }),
    )?;

    activate_and_check_device_path(&mut crypt_device, &activation_name)
}

/// Activate encrypted Stratis device using the name stored in the
/// Stratis token
pub fn activate_encrypted_stratis_device(physical_path: &Path) -> Result<PathBuf> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    crypt_device
        .context_handle()
        .load::<()>(EncryptionFormat::Luks2, None)?;

    let stratis_device_name = get_stratis_device_name(&mut crypt_device)?;

    activate_and_check_device_path(&mut crypt_device, &stratis_device_name)
}
