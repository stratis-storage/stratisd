// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::Debug,
    fs::{File, OpenOptions},
    io::Write,
    iter::once,
    path::{Path, PathBuf},
};

use either::Either;
use rand::{distr::Alphanumeric, rng, Rng};
use serde_json::Value;

use devicemapper::{Bytes, Device, DmName, DmNameBuf, Sectors, IEC};
use libblkid_rs::BlkidProbe;
use libcryptsetup_rs::{
    c_uint,
    consts::{
        flags::{CryptActivate, CryptReencrypt, CryptVolumeKey},
        vals::{
            CryptReencryptDirectionInfo, CryptReencryptModeInfo, EncryptionFormat, KeyslotsSize,
            MetadataSize,
        },
    },
    CryptDevice, CryptInit, CryptParamsLuks2, CryptParamsLuks2Ref, CryptParamsReencrypt,
    SafeMemHandle, TokenInput,
};

#[cfg(test)]
use crate::engine::strat_engine::crypt::shared::ensure_inactive;
use crate::{
    engine::{
        engine::MAX_STRATIS_PASS_SIZE,
        strat_engine::{
            backstore::{backstore::v2, get_devno_from_path},
            cmd::{clevis_luks_bind, clevis_luks_regen, clevis_luks_unbind},
            crypt::{
                consts::{
                    DEFAULT_CRYPT_DATA_OFFSET_V2, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                    DEFAULT_CRYPT_METADATA_SIZE_V2, STRATIS_MEK_SIZE,
                },
                shared::{
                    acquire_crypt_device, activate, activate_by_token, add_keyring_keyslot,
                    clevis_info_from_json, device_from_physical_path,
                    encryption_info_from_metadata, ensure_wiped, get_keyslot_number,
                    get_passphrase, handle_do_reencrypt, handle_setup_reencrypt,
                    interpret_clevis_config, wipe_fallback,
                },
            },
            device::blkdev_size,
            dm::DEVICEMAPPER_PATH,
            names::format_crypt_backstore_name,
            thinpool::ThinPool,
        },
        types::{
            DevicePath, EncryptionInfo, InputEncryptionInfo, KeyDescription, PoolUuid,
            SizedKeyMemory, TokenUnlockMethod, UnlockMechanism,
        },
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
    let encryption_info = encryption_info_from_metadata(device)?;

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
        let all_tokens = encryption_info_from_metadata(device)?.all_infos().count();
        if all_tokens < 1 {
            return Err(StratisError::Msg(
                "Device appears to be missing some of the required Stratis LUKS2 tokens"
                    .to_string(),
            ));
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
    pool_uuid: PoolUuid,
    token_slot: TokenUnlockMethod,
    passphrase: Option<&SizedKeyMemory>,
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
            &metadata.encryption_info,
            token_slot.get_token_slot()?,
            passphrase,
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
        encryption_info: &InputEncryptionInfo,
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
        Self::initialize_with_err(&mut device, physical_path, pool_uuid, encryption_info, luks2_params.as_ref())
            .and_then(|_| {
                let encryption_info = encryption_info_from_metadata(&mut device)?;

                let device_path = DevicePath::new(physical_path)?;
                let activated_path = once(DEVICEMAPPER_PATH).chain(once(activation_name.to_string().as_str())).collect::<PathBuf>();
                let devno = get_devno_from_path(&activated_path)?;
                let size = blkdev_size(&File::open(&activated_path)?)?.sectors();
                Ok(CryptHandle::new(
                    device_path,
                    pool_uuid,
                    encryption_info,
                    devno,
                    size,
                ))
            })
            .map_err(|e| {
                if let Err(e) = device.context_handle().load::<()>(Some(EncryptionFormat::Luks2), None) {
                    warn!("Failed to reload state of encrypted device: {e}");
                    return StratisError::from(e);
                }
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

    /// Format the device and initialize the unlock methods.
    fn initialize_unlock_methods(
        device: &mut CryptDevice,
        physical_path: &Path,
        encryption_info: &InputEncryptionInfo,
        luks2_params: Option<&CryptParamsLuks2>,
    ) -> StratisResult<EncryptionInfo> {
        let mut luks2_params_ref: Option<CryptParamsLuks2Ref<'_>> =
            luks2_params.map(|lp| lp.try_into()).transpose()?;

        device.settings_handle().set_metadata_size(
            MetadataSize::try_from(convert_int!(*DEFAULT_CRYPT_METADATA_SIZE_V2, u128, u64)?)?,
            KeyslotsSize::try_from(convert_int!(*DEFAULT_CRYPT_KEYSLOTS_SIZE, u128, u64)?)?,
        )?;
        device.set_data_offset(*DEFAULT_CRYPT_DATA_OFFSET_V2)?;

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
            *DEFAULT_CRYPT_DATA_OFFSET_V2,
            device.status_handle().get_data_offset()
        );

        let (key_descs, clevis_infos, key_descs_with_token_id, clevis_infos_with_token_id) =
            encryption_info.into_parts()?;

        let (to_delete, clevis_bind_input) = match key_descs_with_token_id.keys().next() {
            Some(t) => (None, Either::Left(*t)),
            _ => {
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
                let keyslot_to_delete = log_on_failure!(
                    device.keyslot_handle().add_by_key(
                        None,
                        None,
                        key.as_ref(),
                        CryptVolumeKey::empty(),
                    ),
                    "Failed to initialize keyslot with provided key in keyring"
                );

                (Some(keyslot_to_delete), Either::Right(key))
            }
        };

        for (token_slot, key_desc) in key_descs_with_token_id {
            add_keyring_keyslot(device, Some(token_slot), key_desc, None)?;
        }
        let has_clevis_with_id = !clevis_infos_with_token_id.is_empty();
        for (token_slot, (pin, config)) in clevis_infos_with_token_id {
            let mut config_to_use = config.to_owned();
            let yes = interpret_clevis_config(pin, &mut config_to_use)?;
            clevis_luks_bind(
                physical_path,
                &clevis_bind_input,
                Some(token_slot),
                pin,
                &config_to_use,
                yes,
            )?;
        }
        if has_clevis_with_id {
            // Need to reload device here to refresh the state of the device
            // after being modified by Clevis.
            device
                .context_handle()
                .load::<()>(Some(EncryptionFormat::Luks2), None)?;
        }

        for key_desc in key_descs {
            add_keyring_keyslot(device, None, key_desc, None)?;
        }
        let has_clevis = !clevis_infos.is_empty();
        for (pin, config) in clevis_infos {
            let mut config_to_use = config.to_owned();
            let yes = interpret_clevis_config(pin, &mut config_to_use)?;
            clevis_luks_bind(
                physical_path,
                &clevis_bind_input,
                None,
                pin,
                &config_to_use,
                yes,
            )?;
        }
        if has_clevis {
            // Need to reload device here to refresh the state of the device
            // after being modified by Clevis.
            device
                .context_handle()
                .load::<()>(Some(EncryptionFormat::Luks2), None)?;
        }

        if let Some(t) = to_delete {
            device.keyslot_handle().destroy(t)?;
        }

        let encryption_info = encryption_info_from_metadata(device)?;

        Ok(encryption_info)
    }

    /// Format the device and initialize the unlock methods, activating the device once it is
    /// successfully set up.
    fn initialize_with_err(
        device: &mut CryptDevice,
        physical_path: &Path,
        pool_uuid: PoolUuid,
        encryption_info: &InputEncryptionInfo,
        luks2_params: Option<&CryptParamsLuks2>,
    ) -> StratisResult<()> {
        let encryption_info =
            Self::initialize_unlock_methods(device, physical_path, encryption_info, luks2_params)?;

        let activation_name = format_crypt_backstore_name(&pool_uuid);
        activate(device, &encryption_info, None, None, &activation_name)
    }

    pub fn rollback(
        device: &mut CryptDevice,
        physical_path: &Path,
        name: &DmName,
    ) -> StratisResult<()> {
        if let Err(e) = ensure_wiped(device, physical_path, name) {
            warn!("Wipe failed: {e}; falling back on manual wipe");
            Err(wipe_fallback(
                physical_path,
                Sectors(0),
                DEFAULT_CRYPT_DATA_OFFSET_V2,
                e,
            ))
        } else {
            Ok(())
        }
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
        token_slot: TokenUnlockMethod,
        passphrase: Option<&SizedKeyMemory>,
    ) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => {
                setup_crypt_handle(device, physical_path, pool_uuid, token_slot, passphrase)
            }
            None => Ok(None),
        }
    }

    /// Load the required information for Stratis from the LUKS2 metadata.
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
    pub fn keyslot(&self, token_id: c_uint) -> StratisResult<Option<c_uint>> {
        get_keyslot_number(&mut self.acquire_crypt_device()?, token_id)
    }

    /// Bind the given device using clevis.
    pub fn bind_clevis(
        &mut self,
        token_slot: Option<u32>,
        pin: &str,
        json: &Value,
    ) -> StratisResult<u32> {
        if self.metadata.encryption_info.all_token_slots().count() >= 15 {
            return Err(StratisError::Msg(
                "Reached limit of 15 token and keyslots for pool".to_string(),
            ));
        }

        let mut json_owned = json.clone();
        let yes = interpret_clevis_config(pin, &mut json_owned)?;

        let old_encryption_info = self.encryption_info().clone();

        let either = get_passphrase(
            &mut acquire_crypt_device(self.luks2_device_path())?,
            self.encryption_info(),
        )?;

        clevis_luks_bind(
            self.luks2_device_path(),
            &either.map_left(|(_, ts, _)| ts).map_right(|(_, key)| key),
            token_slot,
            pin,
            &json_owned,
            yes,
        )?;
        self.metadata.encryption_info =
            encryption_info_from_metadata(&mut acquire_crypt_device(self.luks2_device_path())?)?;
        let new_slot = self
            .metadata
            .encryption_info
            .diff(&old_encryption_info)
            .expect("just added a token slot");

        Ok(new_slot)
    }

    /// Unbind the given device using clevis.
    ///
    /// Precondition: The caller should check that the slot is not empty and that a Clevis token
    /// has been passed as the token slot input.
    pub fn unbind_clevis(&mut self, token_slot: u32) -> StratisResult<()> {
        if self.metadata.encryption_info.all_token_slots().count() < 2 {
            return Err(StratisError::Msg(
                "There is one remaining unlock method; removing the Clevis binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let keyslot = self.keyslot(token_slot)?.ok_or_else(|| {
            StratisError::Msg(format!(
                "Token slot {token_slot} appears to be empty; could not determine keyslots"
            ))
        })?;
        log_on_failure!(
            clevis_luks_unbind(self.luks2_device_path(), keyslot),
            "Failed to unbind device {} from Clevis",
            self.luks2_device_path().display()
        );
        self.metadata.encryption_info.remove(token_slot);
        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    ///
    /// This method needs to re-read the cached Clevis information because
    /// the config may change specifically in the case where a new thumbprint
    /// is provided if Tang keys are rotated.
    pub fn rebind_clevis(&mut self, token_slot: u32) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;
        let keyslot = get_keyslot_number(&mut device, token_slot)?.ok_or_else(|| {
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

        let token = device.token_handle().json_get(token_slot)?;
        let (pin, config) = clevis_info_from_json(&token)?;
        self.metadata
            .encryption_info
            .set_info(token_slot, UnlockMechanism::ClevisInfo((pin, config)))?;
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn bind_keyring(
        &mut self,
        token_slot: Option<u32>,
        key_desc: &KeyDescription,
    ) -> StratisResult<u32> {
        if self.metadata.encryption_info.all_token_slots().count() >= 15 {
            return Err(StratisError::Msg(
                "Reached limit of 15 token and keyslots for pool".to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let key = get_passphrase(&mut device, self.encryption_info())?
            .either(|(_, _, key)| key, |(_, key)| key);

        let t = match token_slot {
            Some(t) => t,
            None => self.metadata.encryption_info.free_token_slot(),
        };

        let token_slot =
            add_keyring_keyslot(&mut device, Some(t), key_desc, Some(Either::Left(key)))?;

        self.metadata
            .encryption_info
            .add_info(t, UnlockMechanism::KeyDesc(key_desc.to_owned()))?;
        Ok(token_slot)
    }

    /// Remove keyring binding from the underlying LUKS2 volume.
    ///
    /// Precondition: The caller should check that the slot is not empty and that a key description token
    /// has been passed as the token slot input.
    pub fn unbind_keyring(&mut self, token_slot: u32) -> StratisResult<()> {
        if self.metadata.encryption_info.all_token_slots().count() < 2 {
            return Err(StratisError::Msg(
                "There is one remaining unlock method; removing the key description binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let mut device = self.acquire_crypt_device()?;
        let keyslot = get_keyslot_number(&mut device, token_slot)?
            .ok_or_else(|| StratisError::Msg("No LUKS2 keyring token was found".to_string()))?;
        device.keyslot_handle().destroy(keyslot)?;
        device
            .token_handle()
            .json_set(TokenInput::RemoveToken(token_slot))?;

        self.metadata.encryption_info.remove(token_slot);
        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    pub fn rebind_keyring(
        &mut self,
        token_slot: u32,
        new_key_desc: &KeyDescription,
    ) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;

        let old_key_description = self.metadata.encryption_info
            .get_info(token_slot)
            .ok_or_else(|| {
                StratisError::Msg("Cannot change passphrase because this device is not bound to a passphrase in the kernel keyring".to_string())
            })?
            .key_desc()
            .ok_or_else(|| {
                StratisError::Msg(format!("Cannot rebind to new key description; token slot {token_slot} is already bound to a Clevis token"))
            })?;
        add_keyring_keyslot(
            &mut device,
            Some(token_slot),
            new_key_desc,
            Some(Either::Right(old_key_description)),
        )?;
        self.metadata.encryption_info.set_info(
            token_slot,
            UnlockMechanism::KeyDesc(new_key_desc.to_owned()),
        )?;
        Ok(())
    }

    /// Encrypt an unencrypted pool.
    pub fn encrypt(
        pool_uuid: PoolUuid,
        thinpool: &mut ThinPool<v2::Backstore>,
        unencrypted_path: &Path,
        encryption_info: &InputEncryptionInfo,
    ) -> StratisResult<Self> {
        let tmp_header = format!("/tmp/temp-header-{pool_uuid}");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp_header)?;
            file.write_all(&[0; 4096])?;
        }

        let mut device = CryptInit::init(Path::new(&tmp_header))?;
        let data_offset = Bytes::from(16 * IEC::Mi).sectors();
        device.set_data_offset(*data_offset)?;

        let sectors = thinpool
            .filesystems()
            .iter()
            .map(|(_, _, fs)| fs.block_size())
            .collect::<StratisResult<Vec<u64>>>()?;
        let min_sector = sectors.iter().min();
        let sector_size = match min_sector {
            Some(min) => convert_int!(*min, u64, u32)?,
            None => {
                let mut probe = BlkidProbe::new_from_filename(unencrypted_path)?;
                let top = probe.get_topology()?;
                convert_int!(top.get_logical_sector_size(), u64, u32)?
            }
        };
        let params = CryptParamsLuks2 {
            data_alignment: 0,
            data_device: None,
            integrity: None,
            integrity_params: None,
            pbkdf: None,
            label: None,
            sector_size,
            subsystem: None,
        };

        let encryption_info = Self::initialize_unlock_methods(
            &mut device,
            Path::new(&tmp_header),
            encryption_info,
            Some(&params),
        )?;
        let (keyslot, key) = get_passphrase(&mut device, &encryption_info)?
            .either(|(keyslot, _, key)| (keyslot, key), |tup| tup);
        device.reencrypt_handle().reencrypt_init_by_passphrase(
            None,
            key.as_ref(),
            None,
            Some(keyslot),
            Some(("aes", "xts-plain")),
            CryptParamsReencrypt {
                mode: CryptReencryptModeInfo::Encrypt,
                direction: CryptReencryptDirectionInfo::Forward,
                resilience: "checksum".to_string(),
                hash: "sha256".to_string(),
                data_shift: 0,
                max_hotzone_size: 0,
                device_size: 0,
                luks2: CryptParamsLuks2 {
                    data_alignment: 0,
                    data_device: None,
                    integrity: None,
                    integrity_params: None,
                    pbkdf: None,
                    label: None,
                    sector_size,
                    subsystem: None,
                },
                flags: CryptReencrypt::INITIALIZE_ONLY,
            },
        )?;

        let mut device = CryptInit::init(unencrypted_path)?;
        device
            .backup_handle()
            .header_restore(Some(EncryptionFormat::Luks2), Path::new(&tmp_header))?;

        let activation_name = &format_crypt_backstore_name(&pool_uuid).to_string();
        device.activate_handle().activate_by_passphrase(
            Some(activation_name),
            None,
            key.as_ref(),
            CryptActivate::SHARED,
        )?;

        device.reencrypt_handle().reencrypt_init_by_passphrase(
            Some(activation_name),
            key.as_ref(),
            None,
            Some(keyslot),
            Some(("aes", "xts-plain")),
            CryptParamsReencrypt {
                mode: CryptReencryptModeInfo::Encrypt,
                direction: CryptReencryptDirectionInfo::Forward,
                resilience: "checksum".to_string(),
                hash: "sha256".to_string(),
                data_shift: 0,
                max_hotzone_size: 0,
                device_size: 0,
                luks2: CryptParamsLuks2 {
                    data_alignment: 0,
                    data_device: None,
                    integrity: None,
                    integrity_params: None,
                    pbkdf: None,
                    label: None,
                    sector_size,
                    subsystem: None,
                },
                flags: CryptReencrypt::RESUME_ONLY,
            },
        )?;
        device.reencrypt_handle().reencrypt2::<()>(None, None)?;

        CryptHandle::setup(unencrypted_path, pool_uuid, TokenUnlockMethod::Any, None)
            .map(|h| h.expect("should have crypt device after online encrypt"))
    }

    /// Prepare the crypt header for reencryption.
    ///
    /// Can be rolled back.
    pub fn setup_reencrypt(&self) -> StratisResult<(u32, SizedKeyMemory, u32)> {
        handle_setup_reencrypt(self.luks2_device_path(), self.encryption_info())
    }

    /// Perform the reencryption.
    ///
    /// Cannot be rolled back.
    pub fn do_reencrypt(&self, slot: u32, key: SizedKeyMemory, new_slot: u32) -> StratisResult<()> {
        handle_do_reencrypt(
            &format_crypt_backstore_name(&self.metadata.pool_uuid).to_string(),
            self.luks2_device_path(),
            slot,
            key,
            new_slot,
        )
    }

    /// Encrypt an unencrypted pool.
    pub fn decrypt(self, pool_uuid: PoolUuid) -> StratisResult<()> {
        let activation_name = format_crypt_backstore_name(&pool_uuid);
        let sector_size = {
            let mut probe = BlkidProbe::new_from_filename(self.luks2_device_path())?;
            let top = probe.get_topology()?;
            convert_int!(top.get_logical_sector_size(), u64, u32)?
        };
        let mut device = acquire_crypt_device(self.luks2_device_path())?;
        let (keyslot, key) = get_passphrase(&mut device, self.encryption_info())?
            .either(|(keyslot, _, key)| (keyslot, key), |tup| tup);
        device.reencrypt_handle().reencrypt_init_by_passphrase(
            Some(&activation_name.to_string()),
            key.as_ref(),
            Some(keyslot),
            None,
            None,
            CryptParamsReencrypt {
                mode: CryptReencryptModeInfo::Decrypt,
                direction: CryptReencryptDirectionInfo::Forward,
                resilience: "checksum".to_string(),
                hash: "sha256".to_string(),
                data_shift: 0,
                max_hotzone_size: 0,
                device_size: 0,
                luks2: CryptParamsLuks2 {
                    data_alignment: 0,
                    data_device: None,
                    integrity: None,
                    integrity_params: None,
                    pbkdf: None,
                    label: None,
                    sector_size,
                    subsystem: None,
                },
                flags: CryptReencrypt::empty(),
            },
        )?;
        device.reencrypt_handle().reencrypt2::<()>(None, None)?;
        Ok(())
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
        activate_by_token(&mut crypt, None, None, CryptActivate::KEYRING_KEY)?;
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
    use libcryptsetup_rs::consts::vals::CryptStatusInfo;

    use crate::engine::{
        strat_engine::{
            crypt::{
                consts::{DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE_V2},
                shared::acquire_crypt_device,
            },
            ns::{unshare_mount_namespace, MemoryFilesystem},
            tests::{crypt, loopbacked, real},
        },
        types::{KeyDescription, PoolUuid},
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

        let ei = InputEncryptionInfo::new(vec![(None, key_description)], vec![])
            .expect("Empty data structure")
            .expect("Passed in key description");
        let result = CryptHandle::initialize(path, pool_uuid, &ei, None);

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

            let ei = InputEncryptionInfo::new(vec![(Some(0), key_desc.clone())], vec![])
                .expect("Empty data structure")
                .expect("Passed in key description");
            let handle = CryptHandle::initialize(path, pool_uuid, &ei, None).unwrap();
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

            let handle = CryptHandle::setup(path, pool_uuid, TokenUnlockMethod::Token(0), None)
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
    // Test passing an unusual, larger sector size for cryptsetup. 4096 should
    // be no smaller than the physical sector size of the loop device, and
    // should be allowed by cryptsetup.
    fn loop_test_set_sector_size() {
        fn the_test(paths: &[&Path]) {
            fn test_set_sector_size(paths: &[&Path], key_description: &KeyDescription) {
                let pool_uuid = PoolUuid::new_v4();

                let ei = InputEncryptionInfo::new(vec![(Some(0), key_description.clone())], vec![])
                    .expect("Empty data structure")
                    .expect("Passed in key description");
                CryptHandle::initialize(paths[0], pool_uuid, &ei, Some(4096u32)).unwrap();
            }

            crypt::insert_and_cleanup_key(paths, test_set_sector_size);
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), the_test);
    }

    fn test_both_initialize(paths: &[&Path]) {
        fn both_initialize(paths: &[&Path], key_desc: &KeyDescription, pool_uuid: PoolUuid) {
            let path = paths.first().copied().expect("Expected exactly one path");
            let ei = InputEncryptionInfo::new(vec![(Some(0), key_desc.clone())], vec![
                (
                    Some(1),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    ),
                )
            ])
            .expect("Empty data structure")
            .expect("Passed in key description and Clevis info");
            let handle = CryptHandle::initialize(path, pool_uuid, &ei, None).unwrap();

            let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
            device.token_handle().json_get(0).unwrap();
            device.token_handle().json_get(1).unwrap();
            handle.deactivate().unwrap();
        }

        fn unlock_clevis(paths: &[&Path], pool_uuid: PoolUuid) {
            let path = paths.first().copied().expect("Expected exactly one path");
            CryptHandle::setup(path, pool_uuid, TokenUnlockMethod::Token(1), None)
                .unwrap()
                .unwrap();
        }

        let pool_uuid = PoolUuid::new_v4();
        crypt::insert_and_remove_key(
            paths,
            |paths, key_desc| both_initialize(paths, key_desc, pool_uuid),
            |paths, _| unlock_clevis(paths, pool_uuid),
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

        let ei = InputEncryptionInfo::new(
            vec![],
            vec![
                (Some(0), (
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                ))
            ]
        )
        .expect("Empty token slot").expect("Input Clevis info");

        let handle = CryptHandle::initialize(path, PoolUuid::new_v4(), &ei, None).unwrap();

        let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
        let (metadata, keyslot) = device.settings_handle().get_metadata_size().unwrap();
        assert_eq!(DEFAULT_CRYPT_METADATA_SIZE_V2, Bytes::from(*metadata));
        assert_eq!(DEFAULT_CRYPT_KEYSLOTS_SIZE, Bytes::from(*keyslot));

        assert!(device.token_handle().json_get(0).is_ok());
        assert!(device.token_handle().json_get(1).is_err());
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

        let ei = InputEncryptionInfo::new(
            vec![],
            vec![(
                Some(0),
                (
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL").expect("TANG_URL env var required")}),
                ),
            )],
        )
        .expect("Empty token slot")
        .expect("Passed in Clevis info");

        assert!(CryptHandle::initialize(path, PoolUuid::new_v4(), &ei, None,).is_err());

        let ei = InputEncryptionInfo::new(
            vec![],
            vec![(
                Some(0),
                (
                    "tang".to_string(),
                    json!({
                        "stratis:tang:trust_url": true,
                        "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                    }),
                ),
            )],
        )
        .expect("Empty token slot")
        .expect("Passed Clevis info");

        CryptHandle::initialize(path, PoolUuid::new_v4(), &ei, None).unwrap();
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

        let ei = InputEncryptionInfo::new(
            vec![],
            vec![
                (
                    Some(0),
                    (
                        "sss".to_string(),
                        json!({"t": 1, "pins": {"tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")}, "tpm2": {}}}),
                    ),
                )
            ]
        )
        .expect("Empty token slot").expect("Passed in Clevis info");

        assert!(CryptHandle::initialize(path, PoolUuid::new_v4(), &ei, None,).is_err());

        let ei = InputEncryptionInfo::new(
            vec![],
            vec![
                (
                    Some(0),
                    (
                        "sss".to_string(),
                        json!({
                            "t": 1,
                            "stratis:tang:trust_url": true,
                            "pins": {
                                "tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")},
                                "tpm2": {}
                            }
                        }),
                    ),
                )
            ]
        )
        .expect("Empty token slot").expect("Input Clevis info");

        CryptHandle::initialize(path, PoolUuid::new_v4(), &ei, None).unwrap();
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
        fn init(paths: &[&Path], pool_uuid: PoolUuid, key_desc: &KeyDescription) {
            let path = paths[0];

            let ei = InputEncryptionInfo::new(vec![(Some(0), key_desc.clone())], vec![])
                .expect("Empty data structure")
                .expect("Input key description");

            let handle = CryptHandle::initialize(path, pool_uuid, &ei, None).unwrap();
            handle.deactivate().unwrap();
        }

        fn unlock(paths: &[&Path], pool_uuid: PoolUuid, key: &SizedKeyMemory) {
            let path = paths[0];

            CryptHandle::setup(path, pool_uuid, TokenUnlockMethod::Any, Some(key))
                .unwrap()
                .unwrap();
        }

        let pool_uuid = PoolUuid::new_v4();
        crypt::insert_and_remove_key(
            paths,
            |paths, key_desc| init(paths, pool_uuid, key_desc),
            |paths, key| unlock(paths, pool_uuid, key),
        );
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
