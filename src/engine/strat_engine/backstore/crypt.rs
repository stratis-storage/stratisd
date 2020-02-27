#![allow(dead_code)]

use std::path::{Path, PathBuf};

use libcryptsetup_rs::{
    c_uint, CryptActivateFlags, CryptDevice, CryptInit, EncryptionFormat, LibcryptErr,
};

type Result<T> = std::result::Result<T, LibcryptErr>;

pub static LUKS2_TOKEN_ID: c_uint = 0;
pub static STRATIS_TOKEN_ID: c_uint = 1;
pub static STRATIS_TOKEN_TYPE: &str = "stratis";

/// Check that the token can open the device.
///
/// No activation will actually occur, only validation.
fn check_luks2_token(crypt_device: &mut CryptDevice) -> Result<()> {
    crypt_device.token_handle().activate_by_token::<()>(
        // FIXME: This will be fixed upon the release of libcryptsetup-rs-0.3.0
        // 
        // "name" should be an optional type
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(std::ptr::null(), 0)) },
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
    json.get("device_name")
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str.to_string())
        .ok_or_else(|| {
            LibcryptErr::Other("Could not get device_name from Stratis token".to_string())
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
        && json.get("device_name").is_some()
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

/// Lay down properly configured LUKS2 metadata on a new physical device
pub fn initialize_encrypted_stratis_device(_physical_path: &Path) -> Result<()> {
    Ok(())
}

pub fn activate_encrypted_stratis_device(physical_path: &Path) -> Result<PathBuf> {
    let mut crypt_device = CryptInit::init(physical_path)?;
    let stratis_device_name = get_stratis_device_name(&mut crypt_device)?;
    crypt_device.token_handle().activate_by_token::<()>(
        stratis_device_name.as_str(),
        Some(LUKS2_TOKEN_ID),
        None,
        CryptActivateFlags::empty(),
    )?;

    // Potentially should just check device activation status but checking
    // that the symlink was created may also be valuable in case a race
    // condition occurs with udev
    let mut activated_path = PathBuf::from("/dev/mapper");
    activated_path.push(stratis_device_name);

    Ok(activated_path)
}
