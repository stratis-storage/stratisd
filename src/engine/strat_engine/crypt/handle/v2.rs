// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::Debug,
    fs::File,
    iter::once,
    path::{Path, PathBuf},
};

use either::Either;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde_json::Value;

use devicemapper::{Device, DmName, DmNameBuf, Sectors};
use libcryptsetup_rs::{
    c_uint,
    consts::{
        flags::{CryptActivate, CryptVolumeKey},
        vals::{EncryptionFormat, KeyslotsSize, MetadataSize},
    },
    CryptDevice, CryptInit, CryptParamsLuks2, CryptParamsLuks2Ref, SafeMemHandle, TokenInput,
};

#[cfg(test)]
use crate::engine::strat_engine::crypt::shared::ensure_inactive;
use crate::{
    engine::{
        engine::MAX_STRATIS_PASS_SIZE,
        strat_engine::{
            backstore::get_devno_from_path,
            cmd::{clevis_decrypt, clevis_luks_bind, clevis_luks_regen, clevis_luks_unbind},
            crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE,
                    LUKS2_TOKEN_ID, STRATIS_MEK_SIZE,
                },
                shared::{
                    acquire_crypt_device, activate, add_keyring_keyslot, clevis_info_from_metadata,
                    device_from_physical_path, ensure_wiped, get_keyslot_number,
                    interpret_clevis_config, key_desc_from_metadata, luks2_token_type_is_valid,
                    wipe_fallback,
                },
            },
            device::blkdev_size,
            dm::DEVICEMAPPER_PATH,
            names::format_crypt_backstore_name,
        },
        types::{
            DevicePath, EncryptionInfo, KeyDescription, PoolUuid, SizedKeyMemory, UnlockMethod,
        },
        ClevisInfo,
    },
    stratis::{StratisError, StratisResult},
};

/// Load crypt device metadata.
pub fn load_crypt_metadata(
    device: &mut CryptDevice,
    physical_path: &Path,
    pool_uuid: PoolUuid,
) -> StratisResult<Option<CryptMetadata>> {
    let physical = DevicePath::new(physical_path)?;

    let activation_name = format_crypt_backstore_name(&pool_uuid);
    let key_description = key_desc_from_metadata(device);
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
        pool_uuid,
        encryption_info,
        activation_name,
        activated_path,
    }))
}

#[derive(Debug, Clone)]
pub struct CryptMetadata {
    pub physical_path: DevicePath,
    pub pool_uuid: PoolUuid,
    pub encryption_info: EncryptionInfo,
    pub activation_name: DmNameBuf,
    pub activated_path: PathBuf,
}

/// Check whether the physical device path corresponds to an encrypted
/// Stratis device.
///
/// This method works on activated and deactivated encrypted devices.
///
/// This device will only return true if the device was initialized
/// with encryption by Stratis. This requires that the device is a LUKS2 encrypted device.
fn is_encrypted_stratis_device(device: &mut CryptDevice) -> bool {
    fn device_operations(device: &mut CryptDevice) -> StratisResult<()> {
        let luks_token = device.token_handle().json_get(LUKS2_TOKEN_ID).ok();
        let clevis_token = device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).ok();
        if luks_token.is_none() && clevis_token.is_none() {
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
pub fn setup_crypt_handle(
    device: &mut CryptDevice,
    physical_path: &Path,
    pool_uuid: PoolUuid,
    unlock_method: UnlockMethod,
) -> StratisResult<Option<CryptHandle>> {
    let metadata = match load_crypt_metadata(device, physical_path, pool_uuid)? {
        Some(m) => m,
        None => return Ok(None),
    };

    if !once(DEVICEMAPPER_PATH)
        .chain(once(metadata.activation_name.to_string().as_str()))
        .collect::<PathBuf>()
        .exists()
    {
        activate(
            device,
            metadata.encryption_info.key_description(),
            unlock_method,
            &metadata.activation_name,
        )?
    }

    let device = get_devno_from_path(&metadata.activated_path)?;
    let size = blkdev_size(&File::open(&metadata.activated_path)?)?.sectors();

    Ok(Some(CryptHandle::new(
        metadata.physical_path,
        metadata.pool_uuid,
        metadata.encryption_info,
        device,
        size,
    )))
}

/// Handle for performing all operations on an encrypted device.
///
/// `Clone` is derived for this data structure because `CryptHandle` acquires
/// a new crypt device context for each operation.
#[derive(Debug, Clone)]
pub struct CryptHandle {
    metadata: CryptMetadata,
    device: Device,
    size: Sectors,
}

impl CryptHandle {
    pub(super) fn new(
        physical_path: DevicePath,
        pool_uuid: PoolUuid,
        encryption_info: EncryptionInfo,
        devno: Device,
        size: Sectors,
    ) -> CryptHandle {
        let activation_name = format_crypt_backstore_name(&pool_uuid);
        let path = vec![DEVICEMAPPER_PATH, &activation_name.to_string()]
            .into_iter()
            .collect::<PathBuf>();
        let activated_path = path.canonicalize().unwrap_or(path);
        CryptHandle {
            metadata: CryptMetadata {
                physical_path,
                pool_uuid,
                encryption_info,
                activation_name,
                activated_path,
            },
            device: devno,
            size,
        }
    }

    /// Initialize a device with the provided key description and Clevis info.
    pub fn initialize(
        physical_path: &Path,
        pool_uuid: PoolUuid,
        encryption_info: &EncryptionInfo,
        sector_size: Option<u32>,
    ) -> StratisResult<Self> {
        let activation_name = format_crypt_backstore_name(&pool_uuid);

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
        Self::initialize_with_err(&mut device, physical_path, pool_uuid, encryption_info, luks2_params.as_ref())
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
                let devno = get_devno_from_path(&once(DEVICEMAPPER_PATH).chain(once(activation_name.to_string().as_str())).collect::<PathBuf>())?;
                let size = blkdev_size(&File::open(["/dev", "mapper", &activation_name.to_string()].iter().collect::<PathBuf>())?)?.sectors();
                Ok(CryptHandle::new(
                    device_path,
                    pool_uuid,
                    encryption_info,
                    devno,
                    size,
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

        let activation_name = format_crypt_backstore_name(&pool_uuid);
        activate(
            device,
            encryption_info.key_description(),
            UnlockMethod::Any,
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
    /// The checks include:
    /// * is a LUKS2 device
    /// * has a valid Stratis LUKS2 token
    /// * has a token of the proper type for LUKS2 keyring unlocking
    pub fn setup(
        physical_path: &Path,
        pool_uuid: PoolUuid,
        unlock_method: UnlockMethod,
    ) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => {
                setup_crypt_handle(device, physical_path, pool_uuid, unlock_method)
            }
            None => Ok(None),
        }
    }

    /// Load the required information for Stratis from the LUKS2 metadata.
    #[cfg(test)]
    pub fn load_metadata(
        physical_path: &Path,
        pool_uuid: PoolUuid,
    ) -> StratisResult<Option<CryptMetadata>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => load_crypt_metadata(device, physical_path, pool_uuid),
            None => Ok(None),
        }
    }

    /// Get the device size for this encrypted device.
    #[cfg(test)]
    pub fn size(&self) -> Sectors {
        self.size
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
    #[cfg(test)]
    pub fn activated_device_path(&self) -> &Path {
        &self.metadata.activated_path
    }

    /// Device number for the LUKS2 encrypted device.
    pub fn device(&self) -> Device {
        self.device
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
    #[cfg(test)]
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

    /// Changed the encrypted device size
    /// `None` will fill up the entire underlying physical device.
    /// `Some(_)` will resize the device to the given number of sectors.
    pub fn resize(&mut self, size: Option<Sectors>) -> StratisResult<()> {
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
            .map_err(StratisError::Crypt)?;
        self.size = blkdev_size(&File::open(&self.metadata.activated_path)?)?.sectors();
        Ok(())
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
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE,
                    LUKS2_TOKEN_ID, STRATIS_MEK_SIZE,
                },
                shared::acquire_crypt_device,
            },
            ns::{unshare_mount_namespace, MemoryFilesystem},
            tests::{crypt, loopbacked, real},
        },
        types::{EncryptionInfo, KeyDescription, PoolUuid, UnlockMethod},
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

        let result = CryptHandle::initialize(
            path,
            pool_uuid,
            &EncryptionInfo::KeyDesc(key_description),
            None,
        );

        // Initialization cannot occur with a non-existent key
        assert!(result.is_err());

        assert!(CryptHandle::load_metadata(path, pool_uuid)
            .unwrap()
            .is_none());

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

            let handle = CryptHandle::initialize(
                path,
                pool_uuid,
                &EncryptionInfo::KeyDesc(key_desc.clone()),
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

            let handle = CryptHandle::setup(path, pool_uuid, UnlockMethod::Keyring)
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
            assert_eq!(DEFAULT_CRYPT_METADATA_SIZE, Bytes::from(*metadata));
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

                CryptHandle::initialize(
                    paths[0],
                    pool_uuid,
                    &EncryptionInfo::KeyDesc(key_description.clone()),
                    Some(4096u32),
                )
                .unwrap();
            }

            crypt::insert_and_cleanup_key(paths, test_set_sector_size);
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), the_test);
    }

    fn test_both_initialize(paths: &[&Path]) {
        fn both_initialize(paths: &[&Path], key_desc: &KeyDescription, pool_uuid: PoolUuid) {
            let path = paths.first().copied().expect("Expected exactly one path");
            let handle = CryptHandle::initialize(
                path,
                pool_uuid,
                &EncryptionInfo::Both(
                    key_desc.clone(),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    ),
                ),
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

        fn unlock_clevis(paths: &[&Path], pool_uuid: PoolUuid) {
            let path = paths.first().copied().expect("Expected exactly one path");
            CryptHandle::setup(path, pool_uuid, UnlockMethod::Clevis)
                .unwrap()
                .unwrap();
        }

        let pool_uuid = PoolUuid::new_v4();
        crypt::insert_and_remove_key(
            paths,
            |paths, key_desc| both_initialize(paths, key_desc, pool_uuid),
            |paths| unlock_clevis(paths, pool_uuid),
        );
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

        let handle = CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            )),
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

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required")}),
            )),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({
                    "stratis:tang:trust_url": true,
                    "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                }),
            )),
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

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            &EncryptionInfo::ClevisInfo((
                "sss".to_string(),
                json!({"t": 1, "pins": {"tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")}, "tpm2": {}}}),
            )),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            &EncryptionInfo::ClevisInfo((
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
}
