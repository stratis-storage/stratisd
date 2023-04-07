// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use either::Either;
use serde_json::{to_value, Value};

use devicemapper::{DmName, DmNameBuf};
use libcryptsetup_rs::{
    consts::{
        flags::CryptVolumeKey,
        vals::{EncryptionFormat, KeyslotsSize, MetadataSize},
    },
    CryptDevice, CryptInit, CryptParamsLuks2, CryptParamsLuks2Ref, TokenInput,
};

use crate::{
    engine::{
        strat_engine::{
            backstore::crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE,
                    STRATIS_MEK_SIZE, STRATIS_TOKEN_ID,
                },
                handle::CryptHandle,
                shared::{
                    activate, add_keyring_keyslot, clevis_info_from_metadata, ensure_wiped,
                    interpret_clevis_config, wipe_fallback, StratisLuks2Token,
                },
            },
            cmd::clevis_luks_bind,
            keys::MemoryPrivateFilesystem,
            metadata::StratisIdentifiers,
            names::format_crypt_name,
        },
        types::{ClevisInfo, DevUuid, DevicePath, EncryptionInfo, KeyDescription, Name, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

/// Handle for initialization actions on a physical device.
pub struct CryptInitializer {
    physical_path: DevicePath,
    identifiers: StratisIdentifiers,
    activation_name: DmNameBuf,
}

impl CryptInitializer {
    pub fn new(
        physical_path: DevicePath,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
    ) -> CryptInitializer {
        CryptInitializer {
            physical_path,
            activation_name: format_crypt_name(&dev_uuid),
            identifiers: StratisIdentifiers::new(pool_uuid, dev_uuid),
        }
    }

    /// Initialize a device with the provided key description and Clevis info.
    pub fn initialize(
        self,
        pool_name: Name,
        key_description: Option<&KeyDescription>,
        clevis_info: Option<&ClevisInfo>,
        sector_size: Option<u32>,
    ) -> StratisResult<CryptHandle> {
        let mut clevis_info_owned =
            clevis_info.map(|(pin, config)| (pin.to_owned(), config.clone()));
        let clevis_parsed = match clevis_info_owned {
            Some((ref pin, ref mut config)) => {
                let yes = interpret_clevis_config(pin, config)?;
                Some((pin.as_str(), &*config, yes))
            }
            None => None,
        };

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
            CryptInit::init(&self.physical_path),
            "Failed to acquire context for device {} while initializing; \
            nothing to clean up",
            self.physical_path.display()
        );
        device.settings_handle().set_metadata_size(
            MetadataSize::try_from(convert_int!(*DEFAULT_CRYPT_METADATA_SIZE, u128, u64)?)?,
            KeyslotsSize::try_from(convert_int!(*DEFAULT_CRYPT_KEYSLOTS_SIZE, u128, u64)?)?,
        )?;
        self
            .initialize_with_err(&mut device, &pool_name, key_description, clevis_parsed, luks2_params.as_ref())
            .and_then(|path| clevis_info_from_metadata(&mut device).map(|ci| (path, ci)))
            .and_then(|(_, clevis_info)| {
                let encryption_info =
                    EncryptionInfo::from_options((key_description.cloned(), clevis_info))
                        .expect("Encrypted device must be provided encryption parameters");
                CryptHandle::new(
                    self.physical_path.clone(),
                    self.identifiers,
                    encryption_info,
                    self.activation_name.clone(),
                    Some(pool_name),
                )
            })
            .map_err(|e| {
                if let Err(err) =
                    Self::rollback(&mut device, &self.physical_path, &self.activation_name)
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
        &self,
        device: &mut CryptDevice,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        let fs = log_on_failure!(
            MemoryPrivateFilesystem::new(),
            "Failed to initialize in memory filesystem for temporary keyfile for
            Clevis binding"
        );
        let keyfile = log_on_failure!(
            fs.rand_key(),
            "Failed to generate a key with random data for Clevis initialization"
        );

        let keyslot = log_on_failure!(
            device.keyslot_handle().add_by_key(
                None,
                None,
                keyfile.as_ref(),
                CryptVolumeKey::empty(),
            ),
            "Failed to initialize keyslot with provided key in keyring"
        );

        clevis_luks_bind(
            &self.physical_path,
            keyfile.keyfile_path(),
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
            return Err(wipe_fallback(&self.physical_path, StratisError::from(e)));
        }

        device.keyslot_handle().destroy(keyslot)?;

        Ok(())
    }

    /// Initialize with both a passphrase in the kernel keyring and Clevis.
    fn initialize_with_both(
        &self,
        device: &mut CryptDevice,
        key_description: &KeyDescription,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        Self::initialize_with_keyring(device, key_description)?;

        let fs = MemoryPrivateFilesystem::new()?;
        fs.key_op(key_description, |kf| {
            clevis_luks_bind(
                &self.physical_path,
                kf,
                CLEVIS_LUKS_TOKEN_ID,
                pin,
                json,
                yes,
            )
        })?;

        // Need to reload device here to refresh the state of the device
        // after being modified by Clevis.
        if let Err(e) = device
            .context_handle()
            .load::<()>(Some(EncryptionFormat::Luks2), None)
        {
            return Err(wipe_fallback(&self.physical_path, StratisError::from(e)));
        }

        Ok(())
    }

    fn initialize_with_err(
        &self,
        device: &mut CryptDevice,
        pool_name: &Name,
        key_description: Option<&KeyDescription>,
        clevis_info: Option<(&str, &Value, bool)>,
        luks2_params: Option<&CryptParamsLuks2>,
    ) -> StratisResult<()> {
        let mut luks2_params_ref: Option<CryptParamsLuks2Ref<'_>> = luks2_params.map(|lp| {
            lp.try_into()
                .expect("the luks2_param struct was constructed by stratisd, so it is valid")
        });

        log_on_failure!(
            device.context_handle().format::<CryptParamsLuks2Ref<'_>>(
                EncryptionFormat::Luks2,
                ("aes", "xts-plain64"),
                None,
                libcryptsetup_rs::Either::Right(STRATIS_MEK_SIZE),
                luks2_params_ref.as_mut(),
            ),
            "Failed to format device {} with LUKS2 header",
            self.physical_path.display()
        );

        match (key_description, clevis_info) {
            (Some(kd), Some(ci)) => self.initialize_with_both(device, kd, ci)?,
            (Some(kd), _) => Self::initialize_with_keyring(device, kd)?,
            (_, Some(ci)) => self.initialize_with_clevis(device, ci)?,
            (_, _) => unreachable!(),
        };

        // Initialize stratis token
        log_on_failure!(
            device.token_handle().json_set(TokenInput::ReplaceToken(
                STRATIS_TOKEN_ID,
                &to_value(&StratisLuks2Token {
                    devname: self.activation_name.clone(),
                    identifiers: self.identifiers,
                    pool_name: Some(pool_name.clone()),
                })?,
            )),
            "Failed to create the Stratis token"
        );

        activate(
            if let Some(kd) = key_description {
                Either::Left((device, kd))
            } else {
                Either::Right(&self.physical_path)
            },
            &self.activation_name,
        )
    }

    pub fn rollback(
        device: &mut CryptDevice,
        physical_path: &Path,
        name: &DmName,
    ) -> StratisResult<()> {
        ensure_wiped(device, physical_path, name)
    }
}
