// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::{self, Formatter},
    fs::OpenOptions,
    io::Write,
    mem::forget,
    path::{Path, PathBuf},
    slice::from_raw_parts_mut,
};

use data_encoding::BASE64URL_NOPAD;
use either::Either;
use serde::{
    de::{Error, MapAccess, Visitor},
    ser::SerializeMap,
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::{from_value, to_value, Map, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use devicemapper::{Bytes, DevId, DmName, DmNameBuf, DmOptions};
use libcryptsetup_rs::{
    c_uint,
    consts::{
        flags::{CryptActivate, CryptVolumeKey, CryptWipe},
        vals::{
            CryptDebugLevel, CryptLogLevel, CryptStatusInfo, CryptWipePattern, EncryptionFormat,
        },
    },
    register, set_debug_level, set_log_callback, CryptDevice, CryptInit, TokenInput,
};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                crypt::{
                    consts::{
                        CLEVIS_LUKS_TOKEN_ID, CLEVIS_RECURSION_LIMIT, CLEVIS_TANG_TRUST_URL,
                        CLEVIS_TOKEN_NAME, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                        DEFAULT_CRYPT_METADATA_SIZE, LUKS2_SECTOR_SIZE, LUKS2_TOKEN_ID,
                        LUKS2_TOKEN_TYPE, STRATIS_TOKEN_DEVNAME_KEY, STRATIS_TOKEN_DEV_UUID_KEY,
                        STRATIS_TOKEN_ID, STRATIS_TOKEN_POOLNAME_KEY, STRATIS_TOKEN_POOL_UUID_KEY,
                        STRATIS_TOKEN_TYPE, TOKEN_KEYSLOTS_KEY, TOKEN_TYPE_KEY,
                    },
                    handle::{CryptHandle, CryptMetadata},
                },
                devices::get_devno_from_path,
            },
            cmd::clevis_decrypt,
            dm::get_dm,
            dm::DEVICEMAPPER_PATH,
            keys,
            metadata::StratisIdentifiers,
        },
        types::{
            DevUuid, DevicePath, EncryptionInfo, KeyDescription, Name, PoolUuid, SizedKeyMemory,
            UnlockMethod,
        },
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

pub struct StratisLuks2Token {
    pub devname: DmNameBuf,
    pub identifiers: StratisIdentifiers,
    pub pool_name: Option<Name>,
}

impl Serialize for StratisLuks2Token {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map_serializer = serializer.serialize_map(None)?;
        map_serializer.serialize_entry(TOKEN_TYPE_KEY, STRATIS_TOKEN_TYPE)?;
        map_serializer.serialize_entry::<_, [u32; 0]>(TOKEN_KEYSLOTS_KEY, &[])?;
        map_serializer.serialize_entry(STRATIS_TOKEN_DEVNAME_KEY, &self.devname.to_string())?;
        map_serializer.serialize_entry(
            STRATIS_TOKEN_POOL_UUID_KEY,
            &self.identifiers.pool_uuid.to_string(),
        )?;
        map_serializer.serialize_entry(
            STRATIS_TOKEN_DEV_UUID_KEY,
            &self.identifiers.device_uuid.to_string(),
        )?;
        if let Some(ref pn) = self.pool_name {
            map_serializer.serialize_entry(STRATIS_TOKEN_POOLNAME_KEY, pn)?;
        }
        map_serializer.end()
    }
}

impl<'de> Deserialize<'de> for StratisLuks2Token {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StratisTokenVisitor;

        impl<'de> Visitor<'de> for StratisTokenVisitor {
            type Value = StratisLuks2Token;

            fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "a Stratis LUKS2 token")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut token_type = None;
                let mut token_keyslots = None;
                let mut d_name = None;
                let mut p_uuid = None;
                let mut d_uuid = None;
                let mut p_name = None;

                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    match k.as_str() {
                        TOKEN_TYPE_KEY => {
                            token_type = Some(v);
                        }
                        TOKEN_KEYSLOTS_KEY => {
                            token_keyslots = Some(v);
                        }
                        STRATIS_TOKEN_DEVNAME_KEY => {
                            d_name = Some(v);
                        }
                        STRATIS_TOKEN_POOL_UUID_KEY => {
                            p_uuid = Some(v);
                        }
                        STRATIS_TOKEN_DEV_UUID_KEY => {
                            d_uuid = Some(v);
                        }
                        STRATIS_TOKEN_POOLNAME_KEY => {
                            p_name = Some(v);
                        }
                        st => {
                            return Err(A::Error::custom(format!("Found unrecognized key {st}")));
                        }
                    }
                }

                token_type
                    .ok_or_else(|| A::Error::custom(format!("Missing field {TOKEN_TYPE_KEY}")))
                    .and_then(|ty| match ty {
                        Value::String(s) => {
                            if s == STRATIS_TOKEN_TYPE {
                                Ok(())
                            } else {
                                Err(A::Error::custom(format!(
                                    "Incorrect value for {TOKEN_TYPE_KEY}: {s}"
                                )))
                            }
                        }
                        _ => Err(A::Error::custom(format!(
                            "Unrecognized value type for {TOKEN_TYPE_KEY}"
                        ))),
                    })
                    .and_then(|_| {
                        let value = token_keyslots.ok_or_else(|| {
                            A::Error::custom(format!("Missing field {TOKEN_KEYSLOTS_KEY}"))
                        })?;
                        match value {
                            Value::Array(a) => {
                                if a.is_empty() {
                                    Ok(())
                                } else {
                                    Err(A::Error::custom(format!(
                                        "Found non-empty array for {TOKEN_KEYSLOTS_KEY}"
                                    )))
                                }
                            }
                            _ => Err(A::Error::custom(format!(
                                "Unrecognized value type for {TOKEN_TYPE_KEY}"
                            ))),
                        }
                    })
                    .and_then(|_| {
                        let value = d_name.ok_or_else(|| {
                            A::Error::custom(format!("Missing field {STRATIS_TOKEN_DEVNAME_KEY}"))
                        })?;
                        match value {
                            Value::String(s) => DmNameBuf::new(s).map_err(A::Error::custom),
                            _ => Err(A::Error::custom(format!(
                                "Unrecognized value type for {STRATIS_TOKEN_DEVNAME_KEY}"
                            ))),
                        }
                    })
                    .and_then(|dev_name| {
                        let value = p_uuid.ok_or_else(|| {
                            A::Error::custom(format!("Missing field {STRATIS_TOKEN_POOL_UUID_KEY}"))
                        })?;
                        match value {
                            Value::String(s) => PoolUuid::parse_str(&s)
                                .map(|uuid| (dev_name, uuid))
                                .map_err(A::Error::custom),
                            _ => Err(A::Error::custom(format!(
                                "Unrecognized value type for {STRATIS_TOKEN_POOL_UUID_KEY}"
                            ))),
                        }
                    })
                    .and_then(|(dev_name, pool_uuid)| {
                        let value = d_uuid.ok_or_else(|| {
                            A::Error::custom(format!("Missing field {STRATIS_TOKEN_DEV_UUID_KEY}"))
                        })?;
                        match value {
                            Value::String(s) => DevUuid::parse_str(&s)
                                .map(|uuid| (dev_name, pool_uuid, uuid))
                                .map_err(A::Error::custom),
                            _ => Err(A::Error::custom(format!(
                                "Unrecognized value type for {STRATIS_TOKEN_DEV_UUID_KEY}"
                            ))),
                        }
                    })
                    .and_then(|(devname, pool_uuid, device_uuid)| {
                        let pool_name = match p_name {
                            Some(Value::String(s)) => Some(Name::new(s)),
                            Some(_) => {
                                return Err(A::Error::custom(format!(
                                    "Unrecognized value type for {STRATIS_TOKEN_POOLNAME_KEY}"
                                )))
                            }
                            None => None,
                        };
                        Ok(StratisLuks2Token {
                            devname,
                            identifiers: StratisIdentifiers {
                                pool_uuid,
                                device_uuid,
                            },
                            pool_name,
                        })
                    })
            }
        }

        deserializer.deserialize_map(StratisTokenVisitor)
    }
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

/// Set up a libcryptsetup device handle on a device that may or may not be a LUKS2
/// device.
pub fn setup_crypt_device(physical_path: &Path) -> StratisResult<Option<CryptDevice>> {
    let device_result = device_from_physical_path(physical_path);
    match device_result {
        Ok(None) => Ok(None),
        Ok(Some(mut dev)) => {
            if !is_encrypted_stratis_device(&mut dev) {
                Ok(None)
            } else {
                Ok(Some(dev))
            }
        }
        Err(e) => Err(e),
    }
}

/// Load crypt device metadata.
pub fn load_crypt_metadata(
    device: &mut CryptDevice,
    physical_path: &Path,
) -> StratisResult<Option<CryptMetadata>> {
    let physical = DevicePath::new(physical_path)?;

    let identifiers = identifiers_from_metadata(device)?;
    let activation_name = activation_name_from_metadata(device)?;
    let pool_name = pool_name_from_metadata(device)?;
    let key_description = key_desc_from_metadata(device);
    let devno = get_devno_from_path(physical_path)?;
    let key_description = match key_description
        .as_ref()
        .map(|kd| KeyDescription::from_system_key_desc(kd))
    {
        Some(Some(Ok(description))) => Some(description),
        Some(Some(Err(e))) => {
            return Err(StratisError::Msg(format!(
                "key description {} found on devnode {} is not a valid Stratis key description: {}",
                key_description.expect("key_desc_from_metadata determined to be Some(_) above"),
                physical_path.display(),
                e,
            )));
        }
        Some(None) => {
            warn!("Key description stored on device {} does not appear to be a Stratis key description; ignoring", physical_path.display());
            None
        }
        None => None,
    };
    let clevis_info = clevis_info_from_metadata(device)?;

    let encryption_info =
        if let Some(info) = EncryptionInfo::from_options((key_description, clevis_info)) {
            info
        } else {
            return Err(StratisError::Msg(format!(
                "No valid encryption method that can be used to unlock device {} found",
                physical_path.display()
            )));
        };

    let path = vec![DEVICEMAPPER_PATH, &activation_name.to_string()]
        .into_iter()
        .collect::<PathBuf>();
    let activated_path = path.canonicalize().unwrap_or(path);
    Ok(Some(CryptMetadata {
        physical_path: physical,
        identifiers,
        encryption_info,
        activation_name,
        pool_name,
        device: devno,
        activated_path,
    }))
}

/// Set up a handle to a crypt device using either Clevis or the keyring to activate
/// the device.
pub fn setup_crypt_handle(
    device: &mut CryptDevice,
    physical_path: &Path,
    unlock_method: Option<UnlockMethod>,
) -> StratisResult<Option<CryptHandle>> {
    let metadata = match load_crypt_metadata(device, physical_path)? {
        Some(m) => m,
        None => return Ok(None),
    };

    if !vec![DEVICEMAPPER_PATH, &metadata.activation_name.to_string()]
        .into_iter()
        .collect::<PathBuf>()
        .exists()
    {
        if let Some(unlock) = unlock_method {
            activate(
                device,
                metadata.encryption_info.key_description(),
                unlock,
                &metadata.activation_name,
            )?;
        }
    }

    Ok(Some(CryptHandle::new(
        metadata.physical_path,
        metadata.identifiers.pool_uuid,
        metadata.identifiers.device_uuid,
        metadata.encryption_info,
        metadata.pool_name,
        metadata.device,
    )))
}

/// Create a device handle and load the LUKS2 header into memory from
/// a physical path.
fn device_from_physical_path(physical_path: &Path) -> StratisResult<Option<CryptDevice>> {
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

    pin_dispatch(&subjson, CLEVIS_RECURSION_LIMIT).map(Some)
}

/// Returns true if the Tang config has a thumbprint or all Tang configs in the nested sss config
/// have thumbprints.
fn all_tang_configs_have_thp(
    pin: &str,
    clevis_config: &Value,
    recursion_limit: u64,
) -> StratisResult<bool> {
    if recursion_limit == 0 {
        return Err(StratisError::Msg(
            "Reached the recursion limit for parsing nested SSS tokens".to_string(),
        ));
    }

    if pin == "tang" {
        if let Some(obj) = clevis_config.as_object() {
            Ok(obj
                .get("thp")
                .map(|val| val.as_str().is_some())
                .unwrap_or(false))
        } else {
            Err(StratisError::Msg(format!(
                "configuration for Clevis is is not in JSON object format: {clevis_config}"
            )))
        }
    } else if pin == "sss" {
        if let Some(obj) = clevis_config.as_object() {
            if let Some(obj) = obj.get("pins").and_then(|val| val.as_object()) {
                obj.iter().try_fold(true, |b, (pin, config)| {
                    Ok(b && all_tang_configs_have_thp(pin, config, recursion_limit - 1)?)
                })
            } else {
                Err(StratisError::Msg(
                    "Unexpected format for Clevis config".to_string(),
                ))
            }
        } else {
            Err(StratisError::Msg(format!(
                "configuration for Clevis is is not in JSON object format: {clevis_config}"
            )))
        }
    } else if pin == "tpm2" {
        Ok(true)
    } else {
        Err(StratisError::Msg(format!("Unrecognized pin {pin}")))
    }
}

/// Interpret non-Clevis keys that may contain additional information about
/// how to configure Clevis when binding. Remove any expected non-Clevis keys
/// from the configuration.
/// The only value to be returned is whether or not the bind command should be
/// passed the argument yes.
pub fn interpret_clevis_config(pin: &str, clevis_config: &mut Value) -> StratisResult<bool> {
    let all_tang_has_thp = all_tang_configs_have_thp(pin, clevis_config, CLEVIS_RECURSION_LIMIT)?;
    let yes = if pin == "tang" || pin == "sss" {
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

    if !all_tang_has_thp && !yes {
        return Err(StratisError::Msg(
            "Either thumbprints for all Tang servers or a directive to trust all Tang servers is required".to_string()
        ));
    }

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
        .find(|&obj| obj.get("key_ops") == Some(&Value::Array(vec![Value::from("verify")])))
        .cloned()
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
fn sss_dispatch(json: &Value, recursion_limit: u64) -> StratisResult<Value> {
    if recursion_limit == 0 {
        return Err(StratisError::Msg(
            "Reached the recursion limit for parsing nested SSS tokens".to_string(),
        ));
    }

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
            let (pin, value) = pin_dispatch(&value, recursion_limit - 1)?;
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
fn pin_dispatch(decoded_jwe: &Value, recursion_limit: u64) -> StratisResult<(String, Value)> {
    let pin_value = decoded_jwe
        .get("clevis")
        .and_then(|map| map.get("pin"))
        .ok_or_else(|| {
            StratisError::Msg("Key .clevis.pin not found in clevis JSON token".to_string())
        })?;
    match pin_value.as_str() {
        Some("tang") => tang_dispatch(decoded_jwe).map(|val| ("tang".to_owned(), val)),
        Some("sss") => {
            sss_dispatch(decoded_jwe, recursion_limit).map(|val| ("sss".to_owned(), val))
        }
        Some("tpm2") => Ok(("tpm2".to_owned(), json!({}))),
        _ => Err(StratisError::Msg("Unsupported clevis pin".to_string())),
    }
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
fn is_encrypted_stratis_device(device: &mut CryptDevice) -> bool {
    fn device_operations(device: &mut CryptDevice) -> StratisResult<()> {
        let stratis_token = device.token_handle().json_get(STRATIS_TOKEN_ID).ok();
        let luks_token = device.token_handle().json_get(LUKS2_TOKEN_ID).ok();
        let clevis_token = device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).ok();
        if stratis_token.is_none() || (luks_token.is_none() && clevis_token.is_none()) {
            return Err(StratisError::Msg(
                "Device appears to be missing some of the required Stratis LUKS2 tokens"
                    .to_string(),
            ));
        }
        if let Some(ref lt) = luks_token {
            if !luks2_token_type_is_valid(lt) {
                return Err(StratisError::Msg("LUKS2 token is invalid".to_string()));
            }
        }
        if let Some(st) = stratis_token {
            if !stratis_token_is_valid(st) {
                return Err(StratisError::Msg("Stratis token is invalid".to_string()));
            }
        }
        Ok(())
    }

    device_operations(device)
        .map(|_| true)
        .map_err(|e| {
            debug!(
                "Operations querying device to determine if it is a Stratis device \
                failed with an error: {}; reporting as not a Stratis device.",
                e
            );
        })
        .unwrap_or(false)
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

/// Activate encrypted Stratis device using the name stored in the
/// Stratis token.
pub fn activate(
    device: &mut CryptDevice,
    key_desc: Option<&KeyDescription>,
    unlock_method: UnlockMethod,
    name: &DmName,
) -> StratisResult<()> {
    if let (Some(kd), UnlockMethod::Keyring) = (key_desc, unlock_method) {
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
    }
    log_on_failure!(
        device.token_handle().activate_by_token::<()>(
            Some(&name.to_string()),
            Some(if unlock_method == UnlockMethod::Keyring {
                LUKS2_TOKEN_ID
            } else {
                CLEVIS_LUKS_TOKEN_ID
            }),
            None,
            CryptActivate::empty(),
        ),
        "Failed to activate device with name {}",
        name
    );

    // Check activation status.
    device_is_active(Some(device), name)?;

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
fn luks2_token_type_is_valid(json: &Value) -> bool {
    json.get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == LUKS2_TOKEN_TYPE)
        .unwrap_or(false)
}

/// Validate that the Stratis token is present and valid
fn stratis_token_is_valid(json: Value) -> bool {
    debug!("Stratis LUKS2 token: {}", json);

    let result = from_value::<StratisLuks2Token>(json);
    if let Err(ref e) = result {
        debug!(
            "LUKS2 token in the Stratis token slot does not appear \
            to be a Stratis token: {}.",
            e,
        );
    }
    result.is_ok()
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

/// Query the Stratis metadata for the device activation name.
fn activation_name_from_metadata(device: &mut CryptDevice) -> StratisResult<DmNameBuf> {
    Ok(from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?.devname)
}

/// Query the Stratis metadata for the key description used to unlock the
/// physical device.
pub fn key_desc_from_metadata(device: &mut CryptDevice) -> Option<String> {
    device.token_handle().luks2_keyring_get(LUKS2_TOKEN_ID).ok()
}

/// Query the Stratis metadata for the pool name.
pub fn pool_name_from_metadata(device: &mut CryptDevice) -> StratisResult<Option<Name>> {
    Ok(
        from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?
            .pool_name,
    )
}

/// Replace the old pool name in the Stratis LUKS2 token.
pub fn replace_pool_name(device: &mut CryptDevice, new_name: Name) -> StratisResult<()> {
    let mut token =
        from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?;
    token.pool_name = Some(new_name);
    device.token_handle().json_set(TokenInput::ReplaceToken(
        STRATIS_TOKEN_ID,
        &to_value(token)?,
    ))?;
    Ok(())
}

/// Query the Stratis metadata for the device identifiers.
fn identifiers_from_metadata(device: &mut CryptDevice) -> StratisResult<StratisIdentifiers> {
    Ok(
        from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?
            .identifiers,
    )
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

fn open_safe(device: &mut CryptDevice, token: libc::c_int) -> StratisResult<SizedKeyMemory> {
    let token = device.token_handle().json_get(token as c_uint).ok();
    let jwe = token.as_ref().and_then(|t| t.get("jwe"));
    if let Some(jwe) = jwe {
        clevis_decrypt(jwe)
    } else {
        Err(StratisError::Msg(format!(
            "Malformed Clevis token: {:?}",
            token.map(|t| t.to_string())
        )))
    }
}

unsafe extern "C" fn open(
    device: *mut libcryptsetup_rs_sys::crypt_device,
    token: libc::c_int,
    buffer: *mut *mut libc::c_char,
    buffer_len: *mut usize,
    _: *mut libc::c_void,
) -> i32 {
    let mut safe_device = CryptDevice::from_ptr(device);
    let res = open_safe(&mut safe_device, token);
    // Required to avoid double free
    forget(safe_device);
    match res {
        Ok(pass) => {
            let malloc_pass = libc::malloc(pass.as_ref().len());
            let pass_slice =
                unsafe { from_raw_parts_mut::<u8>(malloc_pass.cast::<u8>(), pass.as_ref().len()) };
            pass_slice.copy_from_slice(pass.as_ref());
            *buffer = malloc_pass.cast::<libc::c_char>();
            *buffer_len = pass.as_ref().len();
            0
        }
        Err(e) => {
            error!("{}", e.to_string());
            -1
        }
    }
}

unsafe extern "C" fn validate(
    _: *mut libcryptsetup_rs_sys::crypt_device,
    _: *const libc::c_char,
) -> i32 {
    0
}

unsafe extern "C" fn dump(_: *mut libcryptsetup_rs_sys::crypt_device, _: *const libc::c_char) {}

/// Register handler for clevis token
pub fn register_clevis_token() -> StratisResult<()> {
    register(
        CLEVIS_TOKEN_NAME,
        Some(open),
        None,
        Some(validate),
        Some(dump),
    )?;
    Ok(())
}
