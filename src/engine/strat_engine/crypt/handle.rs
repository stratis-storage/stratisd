// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use either::Either;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde_json::{to_value, Value};

use devicemapper::{Device, DmName, DmNameBuf, Sectors};
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
            cmd::{clevis_decrypt, clevis_luks_bind, clevis_luks_regen, clevis_luks_unbind},
            crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE,
                    LUKS2_TOKEN_ID, STRATIS_MEK_SIZE, STRATIS_TOKEN_ID,
                },
                shared::{
                    acquire_crypt_device, activate, add_keyring_keyslot, check_luks2_token,
                    clevis_info_from_metadata, ensure_inactive, ensure_wiped, get_keyslot_number,
                    interpret_clevis_config, key_desc_from_metadata, load_crypt_metadata,
                    replace_pool_name, setup_crypt_device, setup_crypt_handle, wipe_fallback,
                    StratisLuks2Token,
                },
            },
            dm::DEVICEMAPPER_PATH,
            metadata::StratisIdentifiers,
            names::format_crypt_name,
        },
        types::{
            DevUuid, DevicePath, EncryptionInfo, KeyDescription, Name, PoolUuid, SizedKeyMemory,
            UnlockMethod,
        },
        ClevisInfo,
    },
    stratis::{StratisError, StratisResult},
};

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
                let key_description = key_desc_from_metadata(&mut device);

                if key_description.is_some() {
                    check_luks2_token(&mut device)?;
                }
            }
            if try_unlock_clevis {
                log_on_failure!(
                    device.token_handle().activate_by_token::<()>(
                        None,
                        Some(CLEVIS_LUKS_TOKEN_ID),
                        None,
                        CryptActivate::empty(),
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
        encryption_info: &EncryptionInfo,
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
            MetadataSize::try_from(convert_int!(*DEFAULT_CRYPT_METADATA_SIZE, u128, u64)?)?,
            KeyslotsSize::try_from(convert_int!(*DEFAULT_CRYPT_KEYSLOTS_SIZE, u128, u64)?)?,
        )?;
        Self::initialize_with_err(&mut device, physical_path, pool_uuid, dev_uuid, &pool_name, encryption_info, luks2_params.as_ref())
            .and_then(|path| clevis_info_from_metadata(&mut device).map(|ci| (path, ci)))
            .and_then(|(_, clevis_info)| {
                let encryption_info =
                    if let Some(info) = EncryptionInfo::from_options((encryption_info.key_description().cloned(), clevis_info)) {
                        info
                    } else {
                        return Err(StratisError::Msg(format!(
                            "No valid encryption method that can be used to unlock device {} found after initialization",
                            physical_path.display()
                        )));
                    };

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
        add_keyring_keyslot(device, key_description, None)?;

        Ok(())
    }

    /// Initialize with Clevis only.
    fn initialize_with_clevis(
        device: &mut CryptDevice,
        physical_path: &Path,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        let (_, key_data) = thread_rng()
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
            Either::Right(key),
            CLEVIS_LUKS_TOKEN_ID,
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
            return Err(wipe_fallback(physical_path, StratisError::from(e)));
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
            Either::Left(LUKS2_TOKEN_ID),
            CLEVIS_LUKS_TOKEN_ID,
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
            return Err(wipe_fallback(physical_path, StratisError::from(e)));
        }

        Ok(())
    }

    fn initialize_with_err(
        device: &mut CryptDevice,
        physical_path: &Path,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        pool_name: &Name,
        encryption_info: &EncryptionInfo,
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

        match encryption_info {
            EncryptionInfo::Both(kd, (pin, config)) => {
                let mut parsed_config = config.clone();
                let y = interpret_clevis_config(pin, &mut parsed_config)?;
                Self::initialize_with_both(device, physical_path, kd, (pin, &parsed_config, y))?
            }
            EncryptionInfo::KeyDesc(kd) => Self::initialize_with_keyring(device, kd)?,
            EncryptionInfo::ClevisInfo((pin, config)) => {
                let mut parsed_config = config.clone();
                let y = interpret_clevis_config(pin, &mut parsed_config)?;
                Self::initialize_with_clevis(device, physical_path, (pin, &parsed_config, y))?
            }
        };

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

        activate(
            device,
            encryption_info.key_description(),
            if matches!(
                encryption_info,
                EncryptionInfo::Both(_, _) | EncryptionInfo::KeyDesc(_)
            ) {
                UnlockMethod::Keyring
            } else {
                UnlockMethod::Clevis
            },
            &activation_name,
        )
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
        unlock_method: Option<UnlockMethod>,
    ) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => setup_crypt_handle(device, physical_path, unlock_method),
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
    pub fn keyslots(&self, token_id: c_uint) -> StratisResult<Option<Vec<c_uint>>> {
        get_keyslot_number(&mut self.acquire_crypt_device()?, token_id)
    }

    /// Get info for the clevis binding.
    pub fn clevis_info(&self) -> StratisResult<Option<ClevisInfo>> {
        clevis_info_from_metadata(&mut self.acquire_crypt_device()?)
    }

    /// Bind the given device using clevis.
    pub fn clevis_bind(&mut self, pin: &str, json: &Value) -> StratisResult<()> {
        let mut json_owned = json.clone();
        let yes = interpret_clevis_config(pin, &mut json_owned)?;

        clevis_luks_bind(
            self.luks2_device_path(),
            Either::Left(LUKS2_TOKEN_ID),
            CLEVIS_LUKS_TOKEN_ID,
            pin,
            &json_owned,
            yes,
        )?;
        self.metadata.encryption_info =
            self.metadata
                .encryption_info
                .clone()
                .set_clevis_info(self.clevis_info()?.ok_or_else(|| {
                    StratisError::Msg(
                        "Clevis reported successfully binding to device but no metadata was found"
                            .to_string(),
                    )
                })?);
        Ok(())
    }

    /// Unbind the given device using clevis.
    pub fn clevis_unbind(&mut self) -> StratisResult<()> {
        if self.metadata.encryption_info.key_description().is_none() {
            return Err(StratisError::Msg(
                "No kernel keyring binding found; removing the Clevis binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let keyslots = self.keyslots(CLEVIS_LUKS_TOKEN_ID)?.ok_or_else(|| {
            StratisError::Msg(format!(
                "Token slot {CLEVIS_LUKS_TOKEN_ID} appears to be empty; could not determine keyslots"
            ))
        })?;
        for keyslot in keyslots {
            log_on_failure!(
                clevis_luks_unbind(self.luks2_device_path(), keyslot),
                "Failed to unbind device {} from Clevis",
                self.luks2_device_path().display()
            );
        }
        self.metadata.encryption_info = self.metadata.encryption_info.clone().unset_clevis_info();
        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    ///
    /// This method needs to re-read the cached Clevis information because
    /// the config may change specifically in the case where a new thumbprint
    /// is provided if Tang keys are rotated.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        if self.metadata.encryption_info.clevis_info().is_none() {
            return Err(StratisError::Msg(
                "No Clevis binding found; cannot regenerate the Clevis binding if the device does not already have a Clevis binding".to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let keyslot = get_keyslot_number(&mut device, CLEVIS_LUKS_TOKEN_ID)?
            .and_then(|vec| vec.into_iter().next())
            .ok_or_else(|| {
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

        let (pin, config) = clevis_info_from_metadata(&mut device)?.ok_or_else(|| {
            StratisError::Msg(format!(
                "Did not find Clevis metadata on device {}",
                self.luks2_device_path().display()
            ))
        })?;
        self.metadata.encryption_info = self
            .metadata
            .encryption_info
            .clone()
            .set_clevis_info((pin, config));
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;
        let key = Self::clevis_decrypt(&mut device)?.ok_or_else(|| {
            StratisError::Msg(
                "The Clevis token appears to have been wiped outside of \
                    Stratis; cannot add a keyring key binding without an existing \
                    passphrase to unlock the device"
                    .to_string(),
            )
        })?;

        add_keyring_keyslot(&mut device, key_desc, Some(Either::Left(key)))?;

        self.metadata.encryption_info = self
            .metadata
            .encryption_info
            .clone()
            .set_key_desc(key_desc.clone());
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn unbind_keyring(&mut self) -> StratisResult<()> {
        if self.metadata.encryption_info.clevis_info().is_none() {
            return Err(StratisError::Msg(
                "No Clevis binding was found; removing the keyring binding would \
                remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let keyslots = get_keyslot_number(&mut device, LUKS2_TOKEN_ID)?
            .ok_or_else(|| StratisError::Msg("No LUKS2 keyring token was found".to_string()))?;
        for keyslot in keyslots {
            log_on_failure!(
                device.keyslot_handle().destroy(keyslot),
                "Failed partway through the kernel keyring unbinding operation \
                which cannot be rolled back; manual intervention may be required"
            )
        }
        device
            .token_handle()
            .json_set(TokenInput::RemoveToken(LUKS2_TOKEN_ID))?;

        self.metadata.encryption_info = self.metadata.encryption_info.clone().unset_key_desc();

        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    pub fn rebind_keyring(&mut self, new_key_desc: &KeyDescription) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;

        let old_key_description = self.metadata.encryption_info
            .key_description()
            .ok_or_else(|| {
                StratisError::Msg("Cannot change passphrase because this device is not bound to a passphrase in the kernel keyring".to_string())
            })?;
        add_keyring_keyslot(
            &mut device,
            new_key_desc,
            Some(Either::Right(old_key_description)),
        )?;
        self.metadata.encryption_info = self
            .metadata
            .encryption_info
            .clone()
            .set_key_desc(new_key_desc.clone());
        Ok(())
    }

    /// Rename the pool in the LUKS2 token.
    pub fn rename_pool_in_metadata(&mut self, pool_name: Name) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;
        replace_pool_name(&mut device, pool_name)
    }

    /// Decrypt a Clevis passphrase and return it securely.
    fn clevis_decrypt(device: &mut CryptDevice) -> StratisResult<Option<SizedKeyMemory>> {
        let mut token = match device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).ok() {
            Some(t) => t,
            None => return Ok(None),
        };
        let jwe = token
            .as_object_mut()
            .and_then(|map| map.remove("jwe"))
            .ok_or_else(|| {
                StratisError::Msg(format!(
                    "Token slot {CLEVIS_LUKS_TOKEN_ID} is occupied but does not appear to be a Clevis \
                        token; aborting"
                ))
            })?;
        clevis_decrypt(&jwe).map(Some)
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
        crypt.token_handle().activate_by_token::<()>(
            None,
            None,
            None,
            CryptActivate::KEYRING_KEY,
        )?;
        crypt
            .context_handle()
            .resize(&self.activation_name().to_string(), processed_size)
            .map_err(StratisError::Crypt)
    }
}
