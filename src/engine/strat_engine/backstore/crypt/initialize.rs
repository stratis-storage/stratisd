// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    convert::TryFrom,
    path::{Path, PathBuf},
};

use either::Either;
use serde_json::Value;

use libcryptsetup_rs::{
    CryptDevice, CryptInit, CryptVolumeKeyFlags, EncryptionFormat, KeyslotsSize, MetadataSize,
    TokenInput,
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
                    acquire_crypt_device, activate, add_keyring_keyslot, clevis_info_from_metadata,
                    ensure_wiped, interpret_clevis_config, StratisLuks2Token,
                },
            },
            cmd::clevis_luks_bind,
            keys::MemoryPrivateFilesystem,
            metadata::StratisIdentifiers,
            names::format_crypt_name,
        },
        types::{DevUuid, EncryptionInfo, KeyDescription, PoolUuid},
    },
    stratis::StratisResult,
};

/// Handle for initialization actions on a physical device.
pub struct CryptInitializer {
    physical_path: PathBuf,
    identifiers: StratisIdentifiers,
    activation_name: String,
}

impl CryptInitializer {
    pub fn new(physical_path: PathBuf, pool_uuid: PoolUuid, dev_uuid: DevUuid) -> CryptInitializer {
        CryptInitializer {
            physical_path,
            activation_name: format_crypt_name(&dev_uuid),
            identifiers: StratisIdentifiers::new(pool_uuid, dev_uuid),
        }
    }

    /// Acquire a crypt device using the registered LUKS2 device path.
    fn acquire_crypt_device(&self) -> StratisResult<CryptDevice> {
        acquire_crypt_device(&self.physical_path)
    }

    /// Initialize a device with the provided key description and Clevis info.
    pub fn initialize(
        self,
        key_description: Option<&KeyDescription>,
        clevis_info: Option<(&str, &Value)>,
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

        let mut device = log_on_failure!(
            CryptInit::init(&self.physical_path),
            "Failed to acquire context for device {} while initializing; \
            nothing to clean up",
            self.physical_path.display()
        );
        device.settings_handle().set_metadata_size(
            MetadataSize::try_from(DEFAULT_CRYPT_METADATA_SIZE)?,
            KeyslotsSize::try_from(DEFAULT_CRYPT_KEYSLOTS_SIZE)?,
        )?;
        let result = self.initialize_with_err(device, key_description, clevis_parsed);
        let mut device = match self.acquire_crypt_device() {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "Failed to roll back crypt device initialization; you \
                    may need to manually wipe this device: {}",
                    e,
                );
                return Err(e);
            }
        };

        result
            .and_then(|activated_path| {
                Ok(CryptHandle::new(
                    self.physical_path.clone(),
                    activated_path,
                    self.identifiers,
                    EncryptionInfo {
                        key_description: key_description.cloned(),
                        clevis_info: clevis_info_from_metadata(&mut device)?,
                    },
                    self.activation_name.clone(),
                ))
            })
            .map_err(|e| {
                if let Err(err) =
                    Self::rollback(&mut device, &self.physical_path, self.activation_name)
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
        &self,
        device: &mut CryptDevice,
        key_description: &KeyDescription,
    ) -> StratisResult<()> {
        add_keyring_keyslot(device, key_description, None)?;

        Ok(())
    }

    /// Initialize with Clevis only.
    fn initialize_with_clevis(
        &self,
        mut device: CryptDevice,
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
                CryptVolumeKeyFlags::empty(),
            ),
            "Failed to initialize keyslot with provided key in keyring"
        );
        drop(device);

        clevis_luks_bind(
            &self.physical_path,
            keyfile.keyfile_path(),
            CLEVIS_LUKS_TOKEN_ID,
            pin,
            json,
            yes,
        )?;

        // Need to reacquire device here to refresh the state of the device
        // after being modified by Clevis.
        let mut device = self.acquire_crypt_device()?;
        device.keyslot_handle().destroy(keyslot)?;

        Ok(())
    }

    /// Initialize with both a passphrase in the kernel keyring and Clevis.
    fn initialize_with_both(
        &self,
        mut device: CryptDevice,
        key_description: &KeyDescription,
        (pin, json, yes): (&str, &Value, bool),
    ) -> StratisResult<()> {
        self.initialize_with_keyring(&mut device, key_description)?;

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

        Ok(())
    }

    fn initialize_with_err(
        &self,
        mut device: CryptDevice,
        key_description: Option<&KeyDescription>,
        clevis_info: Option<(&str, &Value, bool)>,
    ) -> StratisResult<PathBuf> {
        log_on_failure!(
            device.context_handle().format::<()>(
                EncryptionFormat::Luks2,
                ("aes", "xts-plain64"),
                None,
                libcryptsetup_rs::Either::Right(STRATIS_MEK_SIZE),
                None,
            ),
            "Failed to format device {} with LUKS2 header",
            self.physical_path.display()
        );

        let mut device = match (key_description, clevis_info) {
            (Some(kd), Some(ci)) => {
                self.initialize_with_both(device, kd, ci)?;
                self.acquire_crypt_device()?
            }
            (Some(kd), _) => {
                self.initialize_with_keyring(&mut device, kd)?;
                device
            }
            (_, Some(ci)) => {
                self.initialize_with_clevis(device, ci)?;
                self.acquire_crypt_device()?
            }
            (_, _) => unreachable!(),
        };

        // Initialize stratis token
        log_on_failure!(
            device.token_handle().json_set(TokenInput::ReplaceToken(
                STRATIS_TOKEN_ID,
                &StratisLuks2Token {
                    devname: self.activation_name.clone(),
                    identifiers: self.identifiers,
                }
                .into(),
            )),
            "Failed to create the Stratis token"
        );

        activate(
            if let Some(kd) = key_description {
                Either::Left((&mut device, kd))
            } else {
                Either::Right(&self.physical_path)
            },
            &self.activation_name,
        )
    }

    pub fn rollback(
        device: &mut CryptDevice,
        physical_path: &Path,
        name: String,
    ) -> StratisResult<()> {
        ensure_wiped(device, physical_path, &name)
    }
}
