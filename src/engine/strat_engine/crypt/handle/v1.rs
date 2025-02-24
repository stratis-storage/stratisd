// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::{self, Debug},
    path::{Path, PathBuf},
};

use either::Either;
use rand::{distr::Alphanumeric, rng, Rng};
use serde::{
    de::{Error, MapAccess, Visitor},
    ser::SerializeMap,
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::{from_value, to_value, Value};

use devicemapper::{Bytes, Device, DmName, DmNameBuf, Sectors};
use libcryptsetup_rs::{
    c_uint,
    consts::{
        flags::{CryptActivate, CryptVolumeKey},
        vals::{EncryptionFormat, KeyslotsSize, MetadataSize},
    },
    CryptDevice, CryptInit, CryptParamsLuks2, CryptParamsLuks2Ref, SafeMemHandle, TokenInput,
};

use crate::{
    engine::{
        engine::MAX_STRATIS_PASS_SIZE,
        strat_engine::{
            backstore::get_devno_from_path,
            cmd::{clevis_luks_bind, clevis_luks_regen, clevis_luks_unbind},
            crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                    DEFAULT_CRYPT_METADATA_SIZE_V1, LUKS2_SECTOR_SIZE, LUKS2_TOKEN_ID,
                    STRATIS_MEK_SIZE, STRATIS_TOKEN_DEVNAME_KEY, STRATIS_TOKEN_DEV_UUID_KEY,
                    STRATIS_TOKEN_ID, STRATIS_TOKEN_POOLNAME_KEY, STRATIS_TOKEN_POOL_UUID_KEY,
                    STRATIS_TOKEN_TYPE, TOKEN_KEYSLOTS_KEY, TOKEN_TYPE_KEY,
                },
                shared::{
                    acquire_crypt_device, activate, activate_by_token, add_keyring_keyslot,
                    check_luks2_token, clevis_decrypt, device_from_physical_path,
                    encryption_info_from_metadata, ensure_inactive, ensure_wiped,
                    get_keyslot_number, handle_do_reencrypt, handle_setup_reencrypt,
                    interpret_clevis_config, luks2_token_type_is_valid, read_key, wipe_fallback,
                },
            },
            dm::DEVICEMAPPER_PATH,
            metadata::StratisIdentifiers,
            names::format_crypt_name,
        },
        types::{
            DevUuid, DevicePath, EncryptionInfo, InputEncryptionInfo, KeyDescription, Name,
            PoolUuid, SizedKeyMemory, TokenUnlockMethod, UnlockMechanism,
        },
    },
    stratis::{StratisError, StratisResult},
};

/// Align the number of bytes to the nearest multiple of `LUKS2_SECTOR_SIZE`
/// above the current value.
fn ceiling_sector_size_alignment(bytes: Bytes) -> Bytes {
    let round = *LUKS2_SECTOR_SIZE - 1;
    Bytes::from((*bytes + round) & !round)
}

// Bytes occupied by crypt metadata
pub fn crypt_metadata_size() -> Bytes {
    2u64 * DEFAULT_CRYPT_METADATA_SIZE_V1
        + ceiling_sector_size_alignment(DEFAULT_CRYPT_KEYSLOTS_SIZE)
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

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// Query the Stratis metadata for the device identifiers.
fn identifiers_from_metadata(device: &mut CryptDevice) -> StratisResult<StratisIdentifiers> {
    Ok(
        from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?
            .identifiers,
    )
}

/// Query the Stratis metadata for the device activation name.
fn activation_name_from_metadata(device: &mut CryptDevice) -> StratisResult<DmNameBuf> {
    Ok(from_value::<StratisLuks2Token>(device.token_handle().json_get(STRATIS_TOKEN_ID)?)?.devname)
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

/// Load crypt device metadata.
pub fn load_crypt_metadata(
    device: &mut CryptDevice,
    physical_path: &Path,
) -> StratisResult<Option<CryptMetadata>> {
    let physical = DevicePath::new(physical_path)?;

    let identifiers = identifiers_from_metadata(device)?;
    let activation_name = activation_name_from_metadata(device)?;
    let pool_name = pool_name_from_metadata(device)?;
    let encryption_info = encryption_info_from_metadata(device)?;
    let devno = get_devno_from_path(physical_path)?;

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

/// Set up a handle to a crypt device using either Clevis or the keyring to activate
/// the device.
fn setup_crypt_handle(
    device: &mut CryptDevice,
    physical_path: &Path,
    unlock_method: TokenUnlockMethod,
    passphrase: Option<&SizedKeyMemory>,
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
        if let Ok(opt) = unlock_method.get_token_slot() {
            activate(
                device,
                &metadata.encryption_info,
                opt,
                passphrase,
                &metadata.activation_name,
            )?
        };
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

#[derive(Debug, Clone)]
pub struct CryptMetadata {
    pub physical_path: DevicePath,
    pub identifiers: StratisIdentifiers,
    pub encryption_info: EncryptionInfo,
    pub activation_name: DmNameBuf,
    pub activated_path: PathBuf,
    pub pool_name: Option<Name>,
    pub device: Device,
}

/// Handle for performing all operations on an encrypted device.
///
/// `Clone` is derived for this data structure because `CryptHandle` acquires
/// a new crypt device context for each operation.
#[derive(Debug, Clone)]
pub struct CryptHandle {
    metadata: CryptMetadata,
}

impl CryptHandle {
    pub(super) fn new(
        physical_path: DevicePath,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        encryption_info: EncryptionInfo,
        pool_name: Option<Name>,
        devno: Device,
    ) -> CryptHandle {
        let activation_name = format_crypt_name(&dev_uuid);
        let path = vec![DEVICEMAPPER_PATH, &activation_name.to_string()]
            .into_iter()
            .collect::<PathBuf>();
        let activated_path = path.canonicalize().unwrap_or(path);
        CryptHandle {
            metadata: CryptMetadata {
                physical_path,
                identifiers: StratisIdentifiers {
                    pool_uuid,
                    device_uuid: dev_uuid,
                },
                encryption_info,
                activation_name,
                pool_name,
                device: devno,
                activated_path,
            },
        }
    }

    /// Check whether the given physical device can be unlocked with the current
    /// environment (e.g. the proper key is in the kernel keyring, the device
    /// is formatted as a LUKS2 device, etc.)
    pub fn can_unlock(
        physical_path: &Path,
        try_unlock_keyring: bool,
        try_unlock_clevis: bool,
    ) -> bool {
        fn can_unlock_with_failures(
            physical_path: &Path,
            try_unlock_keyring: bool,
            try_unlock_clevis: bool,
        ) -> StratisResult<bool> {
            let mut device = acquire_crypt_device(physical_path)?;

            if try_unlock_keyring {
                let encryption_info = encryption_info_from_metadata(&mut device)?;
                let key_description = encryption_info.single_key_description();

                if key_description.is_some() {
                    check_luks2_token(&mut device)?;
                }
            }
            if try_unlock_clevis {
                log_on_failure!(
                    activate_by_token(
                        &mut device,
                        None,
                        Some(CLEVIS_LUKS_TOKEN_ID),
                        CryptActivate::empty()
                    ),
                    "libcryptsetup reported that the decrypted Clevis passphrase \
                    is unable to open the encrypted device"
                );
            }
            Ok(true)
        }

        can_unlock_with_failures(physical_path, try_unlock_keyring, try_unlock_clevis)
            .map_err(|e| {
                warn!(
                    "stratisd was unable to simulate opening the given device \
                    in the current environment: {}",
                    e,
                );
            })
            .unwrap_or(false)
    }

    /// Initialize a device with the provided key description and Clevis info.
    pub fn initialize(
        physical_path: &Path,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        pool_name: Name,
        encryption_info: &InputEncryptionInfo,
        sector_size: Option<u32>,
    ) -> StratisResult<Self> {
        let activation_name = format_crypt_name(&dev_uuid);

        let luks2_params = sector_size.map(|sector_size| CryptParamsLuks2 {
            pbkdf: None,
            integrity: None,
            integrity_params: None,
            data_alignment: 0,
            data_device: None,
            sector_size,
            label: None,
            subsystem: None,
        });

        let mut device = log_on_failure!(
            CryptInit::init(physical_path),
            "Failed to acquire context for device {} while initializing; \
            nothing to clean up",
            physical_path.display()
        );
        device.settings_handle().set_metadata_size(
            MetadataSize::try_from(convert_int!(*DEFAULT_CRYPT_METADATA_SIZE_V1, u128, u64)?)?,
            KeyslotsSize::try_from(convert_int!(*DEFAULT_CRYPT_KEYSLOTS_SIZE, u128, u64)?)?,
        )?;
        Self::initialize_with_err(&mut device, physical_path, pool_uuid, dev_uuid, &pool_name, encryption_info, luks2_params.as_ref())
            .and_then(|_| encryption_info_from_metadata(&mut device))
            .and_then(|encryption_info| {
                let device_path = DevicePath::new(physical_path)?;
                let devno = get_devno_from_path(physical_path)?;
                Ok(CryptHandle::new(
                    device_path,
                    pool_uuid,
                    dev_uuid,
                    encryption_info,
                    Some(pool_name),
                    devno,
                ))
            })
            .map_err(|e| {
                if let Err(err) =
                    Self::rollback(&mut device, physical_path, &activation_name)
                {
                    warn!(
                        "Failed to roll back crypt device initialization; you may need to manually wipe this device: {}",
                        err
                    );
                }
                e
            })
    }

    /// Initialize with a passphrase in the kernel keyring only.
    fn initialize_with_keyring(
        device: &mut CryptDevice,
        key_description: &KeyDescription,
    ) -> StratisResult<()> {
        add_keyring_keyslot(device, Some(LUKS2_TOKEN_ID), key_description, None)?;

        Ok(())
    }

    /// Initialize with Clevis only.
    fn initialize_with_clevis(
        device: &mut CryptDevice,
        physical_path: &Path,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        let (_, key_data) = rng()
            .sample_iter(Alphanumeric)
            .take(MAX_STRATIS_PASS_SIZE)
            .fold(
                (0, SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?),
                |(idx, mut mem), ch| {
                    mem.as_mut()[idx] = ch;
                    (idx + 1, mem)
                },
            );

        let key = SizedKeyMemory::new(key_data, MAX_STRATIS_PASS_SIZE);
        let keyslot = log_on_failure!(
            device
                .keyslot_handle()
                .add_by_key(None, None, key.as_ref(), CryptVolumeKey::empty(),),
            "Failed to initialize keyslot with provided key in keyring"
        );

        clevis_luks_bind(
            physical_path,
            &Either::Right(key),
            Some(CLEVIS_LUKS_TOKEN_ID),
            pin,
            json,
            yes,
        )?;

        // Need to reload device here to refresh the state of the device
        // after being modified by Clevis.
        if let Err(e) = device
            .context_handle()
            .load::<()>(Some(EncryptionFormat::Luks2), None)
        {
            return Err(wipe_fallback(
                physical_path,
                Sectors(0),
                crypt_metadata_size().sectors(),
                StratisError::from(e),
            ));
        }

        device.keyslot_handle().destroy(keyslot)?;

        Ok(())
    }

    /// Initialize with both a passphrase in the kernel keyring and Clevis.
    fn initialize_with_both(
        device: &mut CryptDevice,
        physical_path: &Path,
        key_description: &KeyDescription,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        Self::initialize_with_keyring(device, key_description)?;

        clevis_luks_bind(
            physical_path,
            &Either::Left(LUKS2_TOKEN_ID),
            Some(CLEVIS_LUKS_TOKEN_ID),
            pin,
            json,
            yes,
        )?;

        // Need to reload device here to refresh the state of the device
        // after being modified by Clevis.
        if let Err(e) = device
            .context_handle()
            .load::<()>(Some(EncryptionFormat::Luks2), None)
        {
            return Err(wipe_fallback(
                physical_path,
                Sectors(0),
                crypt_metadata_size().sectors(),
                StratisError::from(e),
            ));
        }

        Ok(())
    }

    fn initialize_with_err(
        device: &mut CryptDevice,
        physical_path: &Path,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        pool_name: &Name,
        encryption_info: &InputEncryptionInfo,
        luks2_params: Option<&CryptParamsLuks2>,
    ) -> StratisResult<()> {
        let mut luks2_params_ref: Option<CryptParamsLuks2Ref<'_>> =
            luks2_params.map(|lp| lp.try_into()).transpose()?;

        log_on_failure!(
            device.context_handle().format::<CryptParamsLuks2Ref<'_>>(
                EncryptionFormat::Luks2,
                ("aes", "xts-plain64"),
                None,
                libcryptsetup_rs::Either::Right(STRATIS_MEK_SIZE),
                luks2_params_ref.as_mut()
            ),
            "Failed to format device {} with LUKS2 header",
            physical_path.display()
        );

        assert_eq!(
            *crypt_metadata_size().sectors(),
            device.status_handle().get_data_offset()
        );

        match encryption_info.into_parts_legacy()? {
            (Some(kd), Some((pin, config))) => {
                let mut parsed_config = config.clone();
                let y = interpret_clevis_config(pin, &mut parsed_config)?;
                Self::initialize_with_both(device, physical_path, kd, (pin, &parsed_config, y))?
            }
            (Some(kd), _) => Self::initialize_with_keyring(device, kd)?,
            (_, Some((pin, config))) => {
                let mut parsed_config = config.clone();
                let y = interpret_clevis_config(pin, &mut parsed_config)?;
                Self::initialize_with_clevis(device, physical_path, (pin, &parsed_config, y))?
            }
            (_, _) => {
                return Err(StratisError::Msg(
                    "Found no available unlock methods for encrypted device".to_string(),
                ))
            }
        }

        let activation_name = format_crypt_name(&dev_uuid);
        // Initialize stratis token
        log_on_failure!(
            device.token_handle().json_set(TokenInput::ReplaceToken(
                STRATIS_TOKEN_ID,
                &to_value(StratisLuks2Token {
                    devname: activation_name.clone(),
                    identifiers: StratisIdentifiers {
                        pool_uuid,
                        device_uuid: dev_uuid
                    },
                    pool_name: Some(pool_name.clone()),
                })?,
            )),
            "Failed to create the Stratis token"
        );

        let encryption_info = encryption_info_from_metadata(device)?;

        activate(device, &encryption_info, None, None, &activation_name)
    }

    pub fn rollback(
        device: &mut CryptDevice,
        physical_path: &Path,
        name: &DmName,
    ) -> StratisResult<()> {
        ensure_wiped(device, physical_path, name)
    }

    /// Acquire the crypt device handle for the physical path in this `CryptHandle`.
    pub(super) fn acquire_crypt_device(&self) -> StratisResult<CryptDevice> {
        acquire_crypt_device(self.luks2_device_path())
    }

    /// Query the device metadata to reconstruct a handle for performing operations
    /// on an existing encrypted device.
    ///
    /// This method will check that the metadata on the given device is
    /// for the LUKS2 format and that the LUKS2 metadata is formatted
    /// properly as a Stratis encrypted device. If it is properly
    /// formatted it will return the device identifiers (pool and device UUIDs).
    ///
    /// NOTE: This will not validate that the proper key is in the kernel
    /// keyring. For that, use `CryptHandle::can_unlock()`.
    ///
    /// The checks include:
    /// * is a LUKS2 device
    /// * has a valid Stratis LUKS2 token
    /// * has a token of the proper type for LUKS2 keyring unlocking
    pub fn setup(
        physical_path: &Path,
        unlock_method: TokenUnlockMethod,
        passphrase: Option<&SizedKeyMemory>,
    ) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => {
                setup_crypt_handle(device, physical_path, unlock_method, passphrase)
            }
            None => Ok(None),
        }
    }

    /// Load the required information for Stratis from the LUKS2 metadata.
    pub fn load_metadata(physical_path: &Path) -> StratisResult<Option<CryptMetadata>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => load_crypt_metadata(device, physical_path),
            None => Ok(None),
        }
    }

    /// Reload the required information for Stratis from the LUKS2 metadata.
    pub fn reload_metadata(&mut self) -> StratisResult<()> {
        match setup_crypt_device(self.luks2_device_path())? {
            Some(ref mut device) => {
                self.metadata = load_crypt_metadata(device, self.luks2_device_path())?.ok_or_else(|| StratisError::Msg("Found no crypt metadata on this device".to_string()))?;
                Ok(())
            }
            None => Err(StratisError::Msg("Expected device to be an encrypted device but could not acquire handle to crypt device".to_string())),
        }
    }

    /// Get the encryption info for this encrypted device.
    pub fn encryption_info(&self) -> &EncryptionInfo {
        &self.metadata.encryption_info
    }

    /// Return the path to the device node of the underlying storage device
    /// for the encrypted device.
    pub fn luks2_device_path(&self) -> &Path {
        &self.metadata.physical_path
    }

    /// Return the name of the activated devicemapper device.
    pub fn activation_name(&self) -> &DmName {
        &self.metadata.activation_name
    }

    /// Return the path of the activated devicemapper device.
    pub fn activated_device_path(&self) -> &Path {
        &self.metadata.activated_path
    }

    /// Return the pool name recorded in the LUKS2 metadata.
    pub fn pool_name(&self) -> Option<&Name> {
        self.metadata.pool_name.as_ref()
    }

    /// Device number for the LUKS2 encrypted device.
    pub fn device(&self) -> &Device {
        &self.metadata.device
    }

    /// Get the Stratis device identifiers for a given encrypted device.
    pub fn device_identifiers(&self) -> &StratisIdentifiers {
        &self.metadata.identifiers
    }

    /// Get the keyslot associated with the given token ID.
    pub fn keyslot(&self, token_id: c_uint) -> StratisResult<Option<c_uint>> {
        get_keyslot_number(&mut self.acquire_crypt_device()?, token_id)
    }

    /// Bind the given device using clevis.
    pub fn clevis_bind(&mut self, pin: &str, json: &Value) -> StratisResult<()> {
        let mut json_owned = json.clone();
        let yes = interpret_clevis_config(pin, &mut json_owned)?;

        clevis_luks_bind(
            self.luks2_device_path(),
            &Either::Left(LUKS2_TOKEN_ID),
            Some(CLEVIS_LUKS_TOKEN_ID),
            pin,
            &json_owned,
            yes,
        )?;
        self.metadata.encryption_info =
            encryption_info_from_metadata(&mut acquire_crypt_device(self.luks2_device_path())?)?;
        Ok(())
    }

    /// Unbind the given device using clevis.
    pub fn clevis_unbind(&mut self) -> StratisResult<()> {
        if self
            .metadata
            .encryption_info
            .single_key_description()
            .is_none()
        {
            return Err(StratisError::Msg(
                "No kernel keyring binding found; removing the Clevis binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let keyslot = self.keyslot(CLEVIS_LUKS_TOKEN_ID)?.ok_or_else(|| {
            StratisError::Msg(format!(
                "Token slot {CLEVIS_LUKS_TOKEN_ID} appears to be empty; could not determine keyslots"
            ))
        })?;
        log_on_failure!(
            clevis_luks_unbind(self.luks2_device_path(), keyslot),
            "Failed to unbind device {} from Clevis",
            self.luks2_device_path().display()
        );
        self.metadata.encryption_info.remove(CLEVIS_LUKS_TOKEN_ID);
        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    ///
    /// This method needs to re-read the cached Clevis information because
    /// the config may change specifically in the case where a new thumbprint
    /// is provided if Tang keys are rotated.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        if self.metadata.encryption_info.single_clevis_info().is_none() {
            return Err(StratisError::Msg(
                "No Clevis binding found; cannot regenerate the Clevis binding if the device does not already have a Clevis binding".to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let keyslot = get_keyslot_number(&mut device, CLEVIS_LUKS_TOKEN_ID)?.ok_or_else(|| {
            StratisError::Msg("Clevis binding found but no keyslot was associated".to_string())
        })?;

        clevis_luks_regen(self.luks2_device_path(), keyslot)?;
        // Need to reload LUKS2 metadata after Clevis metadata modification.
        if let Err(e) = device
            .context_handle()
            .load::<()>(Some(EncryptionFormat::Luks2), None)
        {
            return Err(StratisError::Chained(
                "Failed to reload crypt device state after modification to Clevis data".to_string(),
                Box::new(StratisError::from(e)),
            ));
        }

        self.metadata.encryption_info = encryption_info_from_metadata(&mut device)?;
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;
        let key = clevis_decrypt(&mut device, CLEVIS_LUKS_TOKEN_ID)?.ok_or_else(|| {
            StratisError::Msg(
                "The Clevis token appears to have been wiped outside of \
                    Stratis; cannot add a keyring key binding without an existing \
                    passphrase to unlock the device"
                    .to_string(),
            )
        })?;

        add_keyring_keyslot(
            &mut device,
            Some(LUKS2_TOKEN_ID),
            key_desc,
            Some(Either::Left(key)),
        )?;

        self.metadata.encryption_info.add_info(
            LUKS2_TOKEN_ID,
            UnlockMechanism::KeyDesc(key_desc.to_owned()),
        )?;
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn unbind_keyring(&mut self) -> StratisResult<()> {
        if self.metadata.encryption_info.single_clevis_info().is_none() {
            return Err(StratisError::Msg(
                "No Clevis binding was found; removing the keyring binding would \
                remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let keyslot = get_keyslot_number(&mut device, LUKS2_TOKEN_ID)?
            .ok_or_else(|| StratisError::Msg("No LUKS2 keyring token was found".to_string()))?;
        device.keyslot_handle().destroy(keyslot)?;
        device
            .token_handle()
            .json_set(TokenInput::RemoveToken(LUKS2_TOKEN_ID))?;

        self.metadata.encryption_info.remove(LUKS2_TOKEN_ID);

        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    pub fn rebind_keyring(&mut self, new_key_desc: &KeyDescription) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;

        let (_, old_key_description) = self.metadata.encryption_info
            .single_key_description()
            .ok_or_else(|| {
                StratisError::Msg("Cannot change passphrase because this device is not bound to a passphrase in the kernel keyring".to_string())
            })?;
        add_keyring_keyslot(
            &mut device,
            Some(LUKS2_TOKEN_ID),
            new_key_desc,
            Some(Either::Right(old_key_description)),
        )?;
        self.metadata.encryption_info.set_info(
            LUKS2_TOKEN_ID,
            UnlockMechanism::KeyDesc(new_key_desc.clone()),
        )?;
        Ok(())
    }

    /// Encrypt an unencrypted pool.
    pub fn setup_reencrypt(&self) -> StratisResult<(u32, SizedKeyMemory, u32)> {
        handle_setup_reencrypt(self.luks2_device_path(), self.encryption_info())
    }

    /// Encrypt an unencrypted pool.
    pub fn do_reencrypt(
        &self,
        single_keyslot: u32,
        single_key: SizedKeyMemory,
        single_new_keyslot: u32,
    ) -> StratisResult<()> {
        handle_do_reencrypt(
            &format_crypt_name(&self.metadata.identifiers.device_uuid).to_string(),
            self.luks2_device_path(),
            single_keyslot,
            single_key,
            single_new_keyslot,
        )
    }

    /// Rename the pool in the LUKS2 token.
    pub fn rename_pool_in_metadata(&mut self, pool_name: Name) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;
        replace_pool_name(&mut device, pool_name)
    }

    /// Deactivate the device referenced by the current device handle.
    pub fn deactivate(&self) -> StratisResult<()> {
        ensure_inactive(&mut self.acquire_crypt_device()?, self.activation_name())
    }

    /// Wipe all LUKS2 metadata on the device safely using libcryptsetup.
    pub fn wipe(&self) -> StratisResult<()> {
        ensure_wiped(
            &mut self.acquire_crypt_device()?,
            self.luks2_device_path(),
            self.activation_name(),
        )
    }

    /// Get the size of the logical device built on the underlying encrypted physical
    /// device. `devicemapper` will return the size in terms of number of sectors.
    pub fn logical_device_size(&self) -> StratisResult<Sectors> {
        let name = self.activation_name().to_owned();
        let active_device = log_on_failure!(
            self.acquire_crypt_device()?
                .runtime_handle(&name.to_string())
                .get_active_device(),
            "Failed to get device size for encrypted logical device"
        );
        Ok(Sectors(active_device.size))
    }

    /// Changed the encrypted device size
    /// `None` will fill up the entire underlying physical device.
    /// `Some(_)` will resize the device to the given number of sectors.
    pub fn resize(&self, size: Option<Sectors>) -> StratisResult<()> {
        let processed_size = match size {
            Some(s) => {
                if s == Sectors(0) {
                    return Err(StratisError::Msg(
                        "Cannot specify a crypt device size of zero".to_string(),
                    ));
                } else {
                    *s
                }
            }
            None => 0,
        };
        let mut crypt = self.acquire_crypt_device()?;
        let passphrase = if let Some((_, kd)) = self.encryption_info().single_key_description() {
            read_key(kd)?.ok_or_else(|| {
                StratisError::Msg("Failed to find key with key description".to_string())
            })?
        } else if self.encryption_info().single_clevis_info().is_some() {
            clevis_decrypt(&mut crypt, CLEVIS_LUKS_TOKEN_ID)?.expect("Already checked token exists")
        } else {
            unreachable!("Must be encrypted")
        };
        crypt.activate_handle().activate_by_passphrase(
            None,
            None,
            passphrase.as_ref(),
            CryptActivate::KEYRING_KEY,
        )?;
        crypt
            .context_handle()
            .resize(&self.activation_name().to_string(), processed_size)
            .map_err(StratisError::Crypt)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::CString,
        fs::{File, OpenOptions},
        io::{self, Read, Write},
        mem::MaybeUninit,
        path::Path,
        ptr, slice,
    };

    use devicemapper::{Bytes, Sectors, IEC};
    use libcryptsetup_rs::{
        consts::vals::{CryptStatusInfo, EncryptionFormat},
        CryptInit, Either,
    };

    use crate::engine::{
        strat_engine::{
            crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                    DEFAULT_CRYPT_METADATA_SIZE_V1, LUKS2_TOKEN_ID, STRATIS_MEK_SIZE,
                },
                shared::acquire_crypt_device,
            },
            ns::{unshare_mount_namespace, MemoryFilesystem},
            tests::{crypt, loopbacked, real},
        },
        types::{DevUuid, KeyDescription, Name, PoolUuid, UnlockMethod},
    };

    use super::*;

    /// If this method is called without a key with the specified key description
    /// in the kernel ring, it should always fail and allow us to test the rollback
    /// of failed initializations.
    fn test_failed_init(paths: &[&Path]) {
        assert_eq!(paths.len(), 1);

        let path = paths.first().expect("There must be exactly one path");
        let key_description =
            KeyDescription::try_from("I am not a key".to_string()).expect("no semi-colons");

        let pool_uuid = PoolUuid::new_v4();
        let pool_name = Name::new("pool_name".to_string());
        let dev_uuid = DevUuid::new_v4();

        let result = CryptHandle::initialize(
            path,
            pool_uuid,
            dev_uuid,
            pool_name,
            InputEncryptionInfo::new_legacy(Some(key_description), None)
                .as_ref()
                .expect("Passed in key description"),
            None,
        );

        // Initialization cannot occur with a non-existent key
        assert!(result.is_err());

        assert!(CryptHandle::load_metadata(path).unwrap().is_none());

        // TODO: Check actual superblock with libblkid
    }

    #[test]
    fn loop_test_failed_init() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_failed_init,
        );
    }

    #[test]
    fn real_test_failed_init() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_failed_init,
        );
    }

    /// Test the method `can_unlock` works on an initialized device in both
    /// active and inactive states.
    fn test_can_unlock(paths: &[&Path]) {
        fn crypt_test(paths: &[&Path], key_desc: &KeyDescription) {
            let mut handles = vec![];

            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            for path in paths {
                let dev_uuid = DevUuid::new_v4();

                let handle = CryptHandle::initialize(
                    path,
                    pool_uuid,
                    dev_uuid,
                    pool_name.clone(),
                    InputEncryptionInfo::new_legacy(Some(key_desc.to_owned()), None)
                        .as_ref()
                        .expect("Passed in key description"),
                    None,
                )
                .unwrap();
                handles.push(handle);
            }

            for path in paths {
                if !CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should be able to be unlocked");
                }
            }

            for handle in handles.iter_mut() {
                handle.deactivate().unwrap();
            }

            for path in paths {
                if !CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should be able to be unlocked");
                }
            }

            for handle in handles.iter_mut() {
                handle.wipe().unwrap();
            }

            for path in paths {
                if CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should no longer be able to be unlocked");
                }
            }
        }

        crypt::insert_and_cleanup_key(paths, crypt_test)
    }

    #[test]
    fn loop_test_can_unlock() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_can_unlock,
        );
    }

    #[test]
    fn real_test_can_unlock() {
        real::test_with_spec(
            &real::DeviceLimits::Range(1, 3, None, None),
            test_can_unlock,
        );
    }

    /// Test initializing and activating an encrypted device using
    /// the utilities provided here.
    ///
    /// The overall format of the test involves generating a random byte buffer
    /// of size 1 MiB, encrypting it on disk, and then ensuring that the plaintext
    /// cannot be found on the encrypted disk by doing a scan of the disk using
    /// a sliding window.
    ///
    /// The sliding window size of 1 MiB was chosen to lower the number of
    /// searches that need to be done compared to a smaller sliding window
    /// and also to decrease the probability of the random sequence being found
    /// on the disk due to leftover data from other tests.
    // TODO: Rewrite libc calls using nix crate.
    fn test_crypt_device_ops(paths: &[&Path]) {
        fn crypt_test(paths: &[&Path], key_desc: &KeyDescription) {
            let path = paths
                .first()
                .expect("This test only accepts a single device");

            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            let dev_uuid = DevUuid::new_v4();

            let handle = CryptHandle::initialize(
                path,
                pool_uuid,
                dev_uuid,
                pool_name,
                InputEncryptionInfo::new_legacy(Some(key_desc.to_owned()), None)
                    .as_ref()
                    .expect("Passed in key description"),
                None,
            )
            .unwrap();
            let logical_path = handle.activated_device_path();

            const WINDOW_SIZE: usize = 1024 * 1024;
            let mut devicenode = OpenOptions::new().write(true).open(logical_path).unwrap();
            let mut random_buffer = vec![0; WINDOW_SIZE].into_boxed_slice();
            File::open("/dev/urandom")
                .unwrap()
                .read_exact(&mut random_buffer)
                .unwrap();
            devicenode.write_all(&random_buffer).unwrap();
            std::mem::drop(devicenode);

            let dev_path_cstring =
                CString::new(path.to_str().expect("Failed to convert path to string")).unwrap();
            let fd = unsafe { libc::open(dev_path_cstring.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                panic!("{}", io::Error::last_os_error());
            }

            let mut stat: MaybeUninit<libc::stat> = MaybeUninit::zeroed();
            let fstat_result = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
            if fstat_result < 0 {
                panic!("{}", io::Error::last_os_error());
            }
            let device_size =
                convert_int!(unsafe { stat.assume_init() }.st_size, libc::off_t, usize).unwrap();
            let mapped_ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    device_size,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if mapped_ptr.is_null() {
                panic!("mmap failed");
            }

            {
                let disk_buffer =
                    unsafe { slice::from_raw_parts(mapped_ptr as *const u8, device_size) };
                for window in disk_buffer.windows(WINDOW_SIZE) {
                    if window == &*random_buffer as &[u8] {
                        unsafe {
                            libc::munmap(mapped_ptr, device_size);
                            libc::close(fd);
                        };
                        panic!("Disk was not encrypted!");
                    }
                }
            }

            unsafe {
                libc::munmap(mapped_ptr, device_size);
                libc::close(fd);
            };

            let device_name = handle.activation_name();
            loop {
                match libcryptsetup_rs::status(
                    Some(&mut handle.acquire_crypt_device().unwrap()),
                    &device_name.to_string(),
                ) {
                    Ok(CryptStatusInfo::Busy) => (),
                    Ok(CryptStatusInfo::Active) => break,
                    Ok(s) => {
                        panic!("Crypt device is in invalid state {s:?}")
                    }
                    Err(e) => {
                        panic!("Checking device status returned error: {e}")
                    }
                }
            }

            handle.deactivate().unwrap();

            let handle = CryptHandle::setup(
                path,
                TokenUnlockMethod::from(Some(UnlockMethod::Keyring)),
                None,
            )
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "Device {} no longer appears to be a LUKS2 device",
                    path.display(),
                )
            });
            handle.wipe().unwrap();
        }

        assert_eq!(paths.len(), 1);

        crypt::insert_and_cleanup_key(paths, crypt_test);
    }

    #[test]
    fn real_test_crypt_device_ops() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(2 * IEC::Mi))),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn loop_test_crypt_metadata_defaults() {
        fn test_defaults(paths: &[&Path]) {
            let mut context = CryptInit::init(paths[0]).unwrap();
            context
                .context_handle()
                .format::<()>(
                    EncryptionFormat::Luks2,
                    ("aes", "xts-plain64"),
                    None,
                    Either::Right(STRATIS_MEK_SIZE),
                    None,
                )
                .unwrap();
            let (metadata, keyslot) = context.settings_handle().get_metadata_size().unwrap();
            assert_eq!(DEFAULT_CRYPT_METADATA_SIZE_V1, Bytes::from(*metadata));
            assert_eq!(DEFAULT_CRYPT_KEYSLOTS_SIZE, Bytes::from(*keyslot));
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), test_defaults);
    }

    #[test]
    // Test passing an unusual, larger sector size for cryptsetup. 4096 should
    // be no smaller than the physical sector size of the loop device, and
    // should be allowed by cryptsetup.
    fn loop_test_set_sector_size() {
        fn the_test(paths: &[&Path]) {
            fn test_set_sector_size(paths: &[&Path], key_description: &KeyDescription) {
                let pool_uuid = PoolUuid::new_v4();
                let pool_name = Name::new("pool_name".to_string());
                let dev_uuid = DevUuid::new_v4();

                CryptHandle::initialize(
                    paths[0],
                    pool_uuid,
                    dev_uuid,
                    pool_name,
                    InputEncryptionInfo::new_legacy(Some(key_description.clone()), None)
                        .as_ref()
                        .expect("Passed in key description"),
                    Some(4096u32),
                )
                .unwrap();
            }

            crypt::insert_and_cleanup_key(paths, test_set_sector_size);
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), the_test);
    }

    fn test_both_initialize(paths: &[&Path]) {
        fn both_initialize(paths: &[&Path], key_desc: &KeyDescription) {
            unshare_mount_namespace().unwrap();
            let _memfs = MemoryFilesystem::new().unwrap();
            let path = paths.first().copied().expect("Expected exactly one path");
            let pool_name = Name::new("pool_name".to_string());
            let handle = CryptHandle::initialize(
                path,
                PoolUuid::new_v4(),
                DevUuid::new_v4(),
                pool_name,
                InputEncryptionInfo::new_legacy(
                    Some(key_desc.clone()),
                    Some((
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    )),
                ).as_ref().expect("Passed in key description and Clevis info"),
                None,
            ).unwrap();

            let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
            device.token_handle().json_get(LUKS2_TOKEN_ID).unwrap();
            device
                .token_handle()
                .json_get(CLEVIS_LUKS_TOKEN_ID)
                .unwrap();
            handle.deactivate().unwrap();
        }

        fn unlock_clevis(paths: &[&Path]) {
            let path = paths.first().copied().expect("Expected exactly one path");
            CryptHandle::setup(
                path,
                TokenUnlockMethod::from(Some(UnlockMethod::Clevis)),
                None,
            )
            .unwrap()
            .unwrap();
        }

        crypt::insert_and_remove_key(paths, both_initialize, |paths, _| unlock_clevis(paths));
    }

    #[test]
    fn clevis_real_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_both_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_both_initialize,
        );
    }

    fn test_clevis_initialize(paths: &[&Path]) {
        unshare_mount_namespace().unwrap();
        let _memfs = MemoryFilesystem::new().unwrap();
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        let handle = CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            InputEncryptionInfo::new_legacy(None, Some((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            ))).as_ref().expect("Passed in Clevis config"),
            None,
        )
        .unwrap();

        let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
        assert!(device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).is_ok());
        assert!(device.token_handle().json_get(LUKS2_TOKEN_ID).is_err());
    }

    #[test]
    fn clevis_real_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_clevis_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_initialize,
        );
    }

    fn test_clevis_tang_configs(paths: &[&Path]) {
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name.clone(),
            InputEncryptionInfo::new_legacy(
                None,
                Some((
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required")}),
                ))
            )
            .as_ref()
            .expect("Passed in Clevis config"),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            InputEncryptionInfo::new_legacy(
                None,
                Some((
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                )),
            )
            .as_ref()
            .expect("Passed in Clevis config"),
            None,
        )
        .unwrap();
    }

    #[test]
    fn clevis_real_test_clevis_tang_configs() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_clevis_tang_configs,
        );
    }

    #[test]
    fn clevis_loop_test_clevis_tang_configs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_tang_configs,
        );
    }

    fn test_clevis_sss_configs(paths: &[&Path]) {
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name.clone(),
            InputEncryptionInfo::new_legacy(
                None,
                Some((
                    "sss".to_string(),
                    json!({"t": 1, "pins": {"tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")}, "tpm2": {}}}),
                )),
            )
            .as_ref()
            .expect("Passed in Clevis config"),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            InputEncryptionInfo::new_legacy(
                None,
                Some((
                    "sss".to_string(),
                    json!({
                        "t": 1,
                        "stratis:tang:trust_url": true,
                        "pins": {
                            "tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")},
                            "tpm2": {}
                        }
                    }),
                )),
            )
            .as_ref()
            .expect("Passed in Clevis config"),
            None,
        )
        .unwrap();
    }

    #[test]
    fn clevis_real_test_clevis_sss_configs() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_clevis_sss_configs,
        );
    }

    #[test]
    fn clevis_loop_test_clevis_sss_configs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_sss_configs,
        );
    }

    fn test_passphrase_unlock(paths: &[&Path]) {
        fn init(paths: &[&Path], key_desc: &KeyDescription) {
            let path = paths[0];

            let handle = CryptHandle::initialize(
                path,
                PoolUuid::new_v4(),
                DevUuid::new_v4(),
                Name::new("pool_name".to_string()),
                InputEncryptionInfo::new_legacy(Some(key_desc.clone()), None)
                    .as_ref()
                    .expect("Passed in key description"),
                None,
            )
            .unwrap();
            handle.deactivate().unwrap();
        }

        fn unlock(paths: &[&Path], key: &SizedKeyMemory) {
            let path = paths[0];

            CryptHandle::setup(
                path,
                TokenUnlockMethod::from(Some(UnlockMethod::Any)),
                Some(key),
            )
            .unwrap()
            .unwrap();
        }

        crypt::insert_and_remove_key(paths, init, unlock);
    }

    #[test]
    fn real_test_passphrase_unlock() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_passphrase_unlock,
        );
    }

    #[test]
    fn loop_test_passphrase_unlock() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_passphrase_unlock,
        );
    }
}
