// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use data_encoding::BASE64URL_NOPAD;
use either::Either;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use devicemapper::{Bytes, DevId, DmName, DmOptions};
use libcryptsetup_rs::{
    c_uint,
    consts::{
        flags::{CryptActivate, CryptVolumeKey, CryptWipe},
        vals::{
            CryptDebugLevel, CryptLogLevel, CryptStatusInfo, CryptWipePattern, EncryptionFormat,
        },
    },
    set_debug_level, set_log_callback, CryptDevice, CryptInit,
};

use crate::{
    engine::{
        strat_engine::{
            cmd::clevis_luks_unlock,
            crypt::consts::{
                CLEVIS_LUKS_TOKEN_ID, CLEVIS_TANG_TRUST_URL, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                DEFAULT_CRYPT_METADATA_SIZE, LUKS2_SECTOR_SIZE, LUKS2_TOKEN_ID, LUKS2_TOKEN_TYPE,
                TOKEN_KEYSLOTS_KEY, TOKEN_TYPE_KEY,
            },
            dm::get_dm,
            keys,
        },
        types::{KeyDescription, SizedKeyMemory},
    },
    stratis::{StratisError, StratisResult},
};

/// Set up crypt logging to log cryptsetup debug information at the trace level.
pub fn set_up_crypt_logging() {
    fn logging_callback(level: CryptLogLevel, msg: &str, _: Option<&mut ()>) {
        match level {
            CryptLogLevel::Verbose | CryptLogLevel::DebugJson | CryptLogLevel::Debug => {
                trace!("{}", msg)
            }
            CryptLogLevel::Normal => info!("{}", msg),
            CryptLogLevel::Error => error!("{}", msg),
        }
    }

    c_logging_callback!(c_logging_callback, (), logging_callback);

    set_debug_level(CryptDebugLevel::All);
    set_log_callback::<()>(Some(c_logging_callback), None);
}

/// Acquire a crypt device handle or return an error. This serves as a wrapper
/// around device_from_physical_path removing the Option type.
pub fn acquire_crypt_device(physical_path: &Path) -> StratisResult<CryptDevice> {
    device_from_physical_path(physical_path)?.ok_or_else(|| {
        StratisError::Msg(format!(
            "Physical device {} underneath encrypted Stratis has been \
                    determined not to be formatted as a LUKS2 Stratis device",
            physical_path.display(),
        ))
    })
}

/// Get the passphrase associated with a given key description.
fn key_desc_to_passphrase(key_description: &KeyDescription) -> StratisResult<SizedKeyMemory> {
    let key_option = log_on_failure!(
        read_key(key_description),
        "Failed to read key with key description {} from keyring",
        key_description.as_application_str()
    );
    if let Some(key) = key_option {
        Ok(key)
    } else {
        Err(StratisError::Msg(format!(
            "Key with key description {} was not found",
            key_description.as_application_str(),
        )))
    }
}

// Precondition: if clevis_pass.is_none(), device must have the volume key stored
// in memory (this is automatically done when formatting a LUKS2 device).
pub fn add_keyring_keyslot(
    device: &mut CryptDevice,
    key_description: &KeyDescription,
    pass: Option<Either<SizedKeyMemory, &KeyDescription>>,
) -> StratisResult<()> {
    let key = key_desc_to_passphrase(key_description)?;
    let keyslot = match pass {
        Some(Either::Left(ref pass)) => {
            log_on_failure!(
                device
                    .keyslot_handle()
                    .add_by_passphrase(None, pass.as_ref(), key.as_ref()),
                "Failed to initialize keyslot with existing Clevis key"
            )
        }
        Some(Either::Right(kd)) => {
            let pass = key_desc_to_passphrase(kd)?;
            log_on_failure!(
                device.keyslot_handle().change_by_passphrase(
                    None,
                    None,
                    pass.as_ref(),
                    key.as_ref()
                ),
                "Failed to change passphrase for encrypted device"
            ) as c_uint
            // The above cast is a work around for bug in libcryptsetup-rs.
            // The change_by_passphrase method should return a c_uint instead
            // of a c_int as a negative error code will be converted into an
            // error type.
        }
        None => {
            log_on_failure!(
                device.keyslot_handle().add_by_key(
                    None,
                    None,
                    key.as_ref(),
                    CryptVolumeKey::empty(),
                ),
                "Failed to initialize keyslot with provided key in keyring"
            )
        }
    };

    log_on_failure!(
        device
            .token_handle()
            .luks2_keyring_set(Some(LUKS2_TOKEN_ID), &key_description.to_system_string()),
        "Failed to initialize the LUKS2 token for driving keyring activation operations"
    );
    log_on_failure!(
        device
            .token_handle()
            .assign_keyslot(LUKS2_TOKEN_ID, Some(keyslot)),
        "Failed to assign the LUKS2 keyring token to the Stratis keyslot"
    );

    Ok(())
}

/// Create a device handle and load the LUKS2 header into memory from
/// a physical path.
pub fn device_from_physical_path(physical_path: &Path) -> StratisResult<Option<CryptDevice>> {
    let mut device = log_on_failure!(
        CryptInit::init(physical_path),
        "Failed to acquire a context for device {}",
        physical_path.display()
    );

    if device
        .context_handle()
        .load::<()>(Some(EncryptionFormat::Luks2), None)
        .is_err()
    {
        Ok(None)
    } else {
        Ok(Some(device))
    }
}

/// Get the Clevis binding information from the device metadata.
///
/// This method returns:
/// * Ok(Some(_)) if a Clevis token was detected
/// * Ok(None) if no token in the Clevis slot was detected or a token was detected
/// but does not appear to be a Clevis token
/// * Err(_) if the token appears to be a Clevis token but is malformed in some way
pub fn clevis_info_from_metadata(
    device: &mut CryptDevice,
) -> StratisResult<Option<(String, Value)>> {
    let json = match device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).ok() {
        Some(j) => j,
        None => return Ok(None),
    };
    let json_b64 = match json
        .get("jwe")
        .and_then(|map| map.get("protected"))
        .and_then(|string| string.as_str())
    {
        Some(s) => s.to_owned(),
        None => return Ok(None),
    };
    let json_bytes = BASE64URL_NOPAD.decode(json_b64.as_bytes())?;

    let subjson: Value = serde_json::from_slice(json_bytes.as_slice())?;

    pin_dispatch(&subjson).map(Some)
}

/// Interpret non-Clevis keys that may contain additional information about
/// how to configure Clevis when binding. Remove any expected non-Clevis keys
/// from the configuration.
/// The only value to be returned is whether or not the bind command should be
/// passed the argument yes.
pub fn interpret_clevis_config(pin: &str, clevis_config: &mut Value) -> StratisResult<bool> {
    let yes = if pin == "tang" {
        if let Some(map) = clevis_config.as_object_mut() {
            map.remove(CLEVIS_TANG_TRUST_URL)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        } else {
            return Err(StratisError::Msg(format!(
                "configuration for Clevis is is not in JSON object format: {clevis_config}"
            )));
        }
    } else {
        false
    };

    Ok(yes)
}

/// Generate tang JSON
fn tang_dispatch(json: &Value) -> StratisResult<Value> {
    let object = json
        .get("clevis")
        .and_then(|map| map.get("tang"))
        .and_then(|val| val.as_object())
        .ok_or_else(|| {
            StratisError::Msg("Expected an object for value of clevis.tang".to_string())
        })?;
    let url = object.get("url").and_then(|s| s.as_str()).ok_or_else(|| {
        StratisError::Msg("Expected a string for value of clevis.tang.url".to_string())
    })?;

    let keys = object
        .get("adv")
        .and_then(|adv| adv.get("keys"))
        .and_then(|keys| keys.as_array())
        .ok_or_else(|| {
            StratisError::Msg("Expected an array for value of clevis.tang.adv.keys".to_string())
        })?;
    let mut key = keys
        .iter()
        .cloned()
        .find(|obj| obj.get("key_ops") == Some(&Value::Array(vec![Value::from("verify")])))
        .ok_or_else(|| {
            StratisError::Msg("Verification key not found in clevis metadata".to_string())
        })?;

    let map = if let Some(m) = key.as_object_mut() {
        m
    } else {
        return Err(StratisError::Msg(
            "Key value is not in JSON object format".to_string(),
        ));
    };
    map.remove("key_ops");
    map.remove("alg");

    let thp = key.to_string();
    let mut hasher = Sha256::new();
    hasher.update(thp.as_bytes());
    let array = hasher.finalize();
    let thp = BASE64URL_NOPAD.encode(array.as_slice());

    Ok(json!({"url": url.to_owned(), "thp": thp}))
}

/// Generate Shamir secret sharing JSON
fn sss_dispatch(json: &Value) -> StratisResult<Value> {
    let object = json
        .get("clevis")
        .and_then(|map| map.get("sss"))
        .and_then(|val| val.as_object())
        .ok_or_else(|| {
            StratisError::Msg("Expected an object for value of clevis.sss".to_string())
        })?;

    let threshold = object
        .get("t")
        .and_then(|val| val.as_u64())
        .ok_or_else(|| {
            StratisError::Msg("Expected an int for value of clevis.sss.t".to_string())
        })?;
    let jwes = object
        .get("jwe")
        .and_then(|val| val.as_array())
        .ok_or_else(|| {
            StratisError::Msg("Expected an array for value of clevis.sss.jwe".to_string())
        })?;

    let mut sss_map = Map::new();
    sss_map.insert("t".to_string(), Value::from(threshold));

    let mut pin_map = Map::new();
    for jwe in jwes {
        if let Value::String(ref s) = jwe {
            // NOTE: Workaround for the on-disk format for Shamir secret sharing
            // as written by clevis. The base64 encoded string delimits the end
            // of the JSON blob with a period.
            let json_s = Some(s.split_once('.').map_or(&**s, |x| x.0)).ok_or_else(|| {
                StratisError::Msg(format!(
                    "Splitting string {s} on character '.' did not result in \
                        at least one string segment."
                ))
            })?;

            let json_bytes = BASE64URL_NOPAD.decode(json_s.as_bytes())?;
            let value: Value = serde_json::from_slice(&json_bytes)?;
            let (pin, value) = pin_dispatch(&value)?;
            match pin_map.get_mut(&pin) {
                Some(Value::Array(ref mut vec)) => vec.push(value),
                None => {
                    pin_map.insert(pin, Value::from(vec![value]));
                }
                _ => {
                    return Err(StratisError::Msg(format!(
                        "There appears to be a data type that is not an array in \
                            the data structure being used to construct the sss JSON config
                            under pin name {pin}"
                    )))
                }
            };
        } else {
            return Err(StratisError::Msg(
                "Expected a string for each value in the array at clevis.sss.jwe".to_string(),
            ));
        }
    }
    sss_map.insert("pins".to_string(), Value::from(pin_map));

    Ok(Value::from(sss_map))
}

/// Match pin for existing JWE
fn pin_dispatch(decoded_jwe: &Value) -> StratisResult<(String, Value)> {
    let pin_value = decoded_jwe
        .get("clevis")
        .and_then(|map| map.get("pin"))
        .ok_or_else(|| {
            StratisError::Msg("Key .clevis.pin not found in clevis JSON token".to_string())
        })?;
    match pin_value.as_str() {
        Some("tang") => tang_dispatch(decoded_jwe).map(|val| ("tang".to_owned(), val)),
        Some("sss") => sss_dispatch(decoded_jwe).map(|val| ("sss".to_owned(), val)),
        Some("tpm2") => Ok(("tpm2".to_owned(), json!({}))),
        _ => Err(StratisError::Msg("Unsupported clevis pin".to_string())),
    }
}

fn device_is_active(device: Option<&mut CryptDevice>, device_name: &DmName) -> StratisResult<()> {
    match libcryptsetup_rs::status(device, &device_name.to_string()) {
        Ok(CryptStatusInfo::Active) => Ok(()),
        Ok(CryptStatusInfo::Busy) => {
            info!(
                "Newly activated device {} reported that it was busy; you may see \
                temporary failures due to the device being busy.",
                device_name,
            );
            Ok(())
        }
        Ok(CryptStatusInfo::Inactive) => {
            warn!(
                "Newly activated device {} reported that it is inactive; device \
                activation appears to have failed",
                device_name,
            );
            Err(StratisError::Msg(format!(
                "Device {device_name} was activated but is reporting that it is inactive"
            )))
        }
        Ok(CryptStatusInfo::Invalid) => {
            warn!(
                "Newly activated device {} reported that its status is invalid; \
                device activation appears to have failed",
                device_name,
            );
            Err(StratisError::Msg(format!(
                "Device {device_name} was activated but is reporting an invalid status"
            )))
        }
        Err(e) => Err(StratisError::Msg(format!(
            "Failed to fetch status for device name {device_name}: {e}"
        ))),
    }
}

/// Activate device by LUKS2 keyring token.
///
/// Precondition: The key description has been verified to be present in the keyring
/// if matches!(unlock_method, UnlockMethod::Keyring).
fn activate_with_keyring(crypt_device: &mut CryptDevice, name: &DmName) -> StratisResult<()> {
    // Activate by token
    log_on_failure!(
        crypt_device.token_handle().activate_by_token::<()>(
            Some(&name.to_string()),
            Some(LUKS2_TOKEN_ID),
            None,
            CryptActivate::empty(),
        ),
        "Failed to activate device with name {}",
        name
    );
    Ok(())
}

/// Activate encrypted Stratis device using the name stored in the
/// Stratis token.
pub fn activate(
    unlock_param: Either<(&mut CryptDevice, &KeyDescription), &Path>,
    name: &DmName,
) -> StratisResult<()> {
    let crypt_device = match unlock_param {
        Either::Left((device, kd)) => {
            let key_description_missing = keys::search_key_persistent(kd)
                .map_err(|_| {
                    StratisError::Msg(format!(
                        "Searching the persistent keyring for the key description {} failed.",
                        kd.as_application_str(),
                    ))
                })?
                .is_none();
            if key_description_missing {
                warn!(
                    "Key description {} was not found in the keyring",
                    kd.as_application_str()
                );
                return Err(StratisError::Msg(format!(
                    "The key description \"{}\" is not currently set.",
                    kd.as_application_str(),
                )));
            }
            activate_with_keyring(device, name)?;
            Some(device)
        }
        Either::Right(path) => {
            clevis_luks_unlock(path, name)?;
            None
        }
    };

    // Check activation status.
    device_is_active(crypt_device, name)?;

    Ok(())
}

/// Get a list of all keyslots associated with the LUKS2 token.
/// This is necessary because attempting to destroy an uninitialized
/// keyslot will result in an error.
pub fn get_keyslot_number(
    device: &mut CryptDevice,
    token_id: c_uint,
) -> StratisResult<Option<Vec<c_uint>>> {
    let json = match device.token_handle().json_get(token_id) {
        Ok(j) => j,
        Err(_) => return Ok(None),
    };
    let vec = json
        .get(TOKEN_KEYSLOTS_KEY)
        .and_then(|k| k.as_array())
        .ok_or_else(|| StratisError::Msg("keyslots value was malformed".to_string()))?;
    Ok(Some(
        vec.iter()
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
                let as_c_uint = s.parse::<c_uint>();
                if let Err(ref e) = as_c_uint {
                    warn!(
                        "Discarding invalid value in LUKS2 token keyslot array: {}; \
                    failed to convert it to an integer: {}",
                        s, e,
                    );
                }
                as_c_uint.ok()
            })
            .collect::<Vec<_>>(),
    ))
}

/// Deactivate an encrypted Stratis device but do not wipe it. This is not
/// a destructive action. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup. This method is idempotent and leaves
/// the state as inactive.
pub fn ensure_inactive(device: &mut CryptDevice, name: &DmName) -> StratisResult<()> {
    let status = log_on_failure!(
        libcryptsetup_rs::status(Some(device), &name.to_string()),
        "Failed to determine status of device with name {}",
        name
    );
    match status {
        CryptStatusInfo::Active | CryptStatusInfo::Busy => {
            get_dm().device_remove(&DevId::Name(name), DmOptions::default())?;
        }
        _ => (),
    }
    Ok(())
}

/// Align the number of bytes to the nearest multiple of `LUKS2_SECTOR_SIZE`
/// above the current value.
fn ceiling_sector_size_alignment(bytes: Bytes) -> Bytes {
    let round = *LUKS2_SECTOR_SIZE - 1;
    Bytes::from((*bytes + round) & !round)
}

/// Fallback method for wiping a crypt device where a handle to the encrypted device
/// cannot be acquired.
pub fn wipe_fallback(path: &Path, causal_error: StratisError) -> StratisError {
    let mut file = match OpenOptions::new().write(true).open(path) {
        Ok(f) => f,
        Err(e) => {
            return StratisError::NoActionRollbackError {
                causal_error: Box::new(causal_error),
                rollback_error: Box::new(StratisError::from(e)),
            }
        }
    };
    let size = match convert_int!(*crypt_metadata_size(), u128, usize) {
        Ok(s) => s,
        Err(e) => {
            return StratisError::NoActionRollbackError {
                causal_error: Box::new(causal_error),
                rollback_error: Box::new(e),
            }
        }
    };
    match file.write_all(vec![0; size].as_slice()) {
        Ok(()) => causal_error,
        Err(e) => StratisError::NoActionRollbackError {
            causal_error: Box::new(causal_error),
            rollback_error: Box::new(StratisError::from(e)),
        },
    }
}

/// Deactivate an encrypted Stratis device and wipe it. This is
/// a destructive action and data will be unrecoverable from this device
/// after this operation. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup. `physical_path` should be the path to
/// the device node of the physical storage backing the encrypted volume.
/// This method is idempotent and leaves the disk as wiped.
pub fn ensure_wiped(
    device: &mut CryptDevice,
    physical_path: &Path,
    name: &DmName,
) -> StratisResult<()> {
    ensure_inactive(device, name)?;
    let keyslot_number = get_keyslot_number(device, LUKS2_TOKEN_ID);
    match keyslot_number {
        Ok(Some(nums)) => {
            for i in nums.iter() {
                log_on_failure!(
                    device.keyslot_handle().destroy(*i),
                    "Failed to destroy keyslot at index {}",
                    i
                );
            }
        }
        Ok(None) => {
            info!(
                "Token ID for keyslots to be wiped appears to be empty; the keyslot \
                area will still be wiped in the next step."
            );
        }
        Err(e) => {
            info!(
                "Keyslot numbers were not found; skipping explicit \
                destruction of keyslots; the keyslot area will still \
                be wiped in the next step: {}",
                e,
            );
        }
    }

    let (md_size, ks_size) = log_on_failure!(
        device.settings_handle().get_metadata_size(),
        "Failed to acquire LUKS2 metadata size"
    );
    debug!("Metadata size of LUKS2 device: {}", *md_size);
    debug!("Keyslot area size of LUKS2 device: {}", *ks_size);
    assert!(*md_size % 4096 == 0);
    let total_luks2_metadata_size = *md_size * 2
        + convert_int!(
            *ceiling_sector_size_alignment(Bytes::from(*ks_size)),
            u128,
            u64
        )?;
    debug!("Aligned total size: {}", total_luks2_metadata_size);

    log_on_failure!(
        device.wipe_handle().wipe::<()>(
            physical_path,
            CryptWipePattern::Zero,
            0,
            total_luks2_metadata_size,
            convert_const!(*LUKS2_SECTOR_SIZE, u128, usize),
            CryptWipe::empty(),
            None,
            None,
        ),
        "Failed to wipe device with name {}",
        name
    );
    Ok(())
}

/// Check that the token can open the device.
///
/// No activation will actually occur, only validation.
pub fn check_luks2_token(device: &mut CryptDevice) -> StratisResult<()> {
    log_on_failure!(
        device.token_handle().activate_by_token::<()>(
            None,
            Some(LUKS2_TOKEN_ID),
            None,
            CryptActivate::empty(),
        ),
        "libcryptsetup reported that the LUKS2 token is unable to \
        open the encrypted device; this could be due to a malformed \
        LUKS2 keyring token on the device or a missing or inaccessible \
        key in the keyring"
    );
    Ok(())
}

/// Validate that the LUKS2 token is present and valid
///
/// May not be necessary. See the comment above the invocation.
pub fn luks2_token_type_is_valid(json: &Value) -> bool {
    json.get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == LUKS2_TOKEN_TYPE)
        .unwrap_or(false)
}

/// Read key from keyring with the given key description.
///
/// Returns a safe owned memory segment that will clear itself when dropped.
///
/// A return result of `Ok(None)` indicates that the key was not found
/// but no error occurred.
///
/// Requires cryptsetup 2.3
pub fn read_key(key_description: &KeyDescription) -> StratisResult<Option<SizedKeyMemory>> {
    let read_key_result = keys::read_key_persistent(key_description);
    if read_key_result.is_err() {
        warn!(
            "Failed to read the key with key description {}; encryption cannot \
            continue",
            key_description.as_application_str(),
        );
    }
    read_key_result.map(|opt| opt.map(|(_, mem)| mem))
}

/// Query the Stratis metadata for the key description used to unlock the
/// physical device.
pub fn key_desc_from_metadata(device: &mut CryptDevice) -> Option<String> {
    device.token_handle().luks2_keyring_get(LUKS2_TOKEN_ID).ok()
}

// Bytes occupied by crypt metadata
pub fn crypt_metadata_size() -> Bytes {
    2u64 * DEFAULT_CRYPT_METADATA_SIZE + ceiling_sector_size_alignment(DEFAULT_CRYPT_KEYSLOTS_SIZE)
}

/// Back up the LUKS2 header to a temporary file.
pub fn back_up_luks_header(dev_path: &Path, tmp_dir: &TempDir) -> StratisResult<PathBuf> {
    let file_name = dev_path.display().to_string().replace('/', "_");
    let pathbuf = vec![tmp_dir.path(), Path::new(&file_name)]
        .into_iter()
        .collect::<PathBuf>();
    acquire_crypt_device(dev_path)?
        .backup_handle()
        .header_backup(Some(EncryptionFormat::Luks2), &pathbuf)?;
    Ok(pathbuf)
}

/// Restore the LUKS2 header from a temporary file.
pub fn restore_luks_header(dev_path: &Path, backup_path: &Path) -> StratisResult<()> {
    acquire_crypt_device(dev_path)?
        .backup_handle()
        .header_restore(Some(EncryptionFormat::Luks2), backup_path)?;
    Ok(())
}