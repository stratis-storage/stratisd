// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fmt::Debug,
    iter::once,
    path::{Path, PathBuf},
};

use either::Either;
use serde_json::Value;

use devicemapper::Sectors;
use libcryptsetup_rs::{c_uint, consts::vals::EncryptionFormat, CryptDevice, TokenInput};

use crate::{
    engine::{
        strat_engine::{
            backstore::crypt::{
                consts::{CLEVIS_LUKS_TOKEN_ID, LUKS2_TOKEN_ID},
                metadata_handle::CryptMetadataHandle,
                shared::{
                    acquire_crypt_device, add_keyring_keyslot, clevis_info_from_metadata,
                    ensure_wiped, get_keyslot_number, interpret_clevis_config, setup_crypt_device,
                    setup_crypt_handle,
                },
            },
            cmd::{clevis_decrypt, clevis_luks_bind, clevis_luks_regen, clevis_luks_unbind},
            dm::DEVICEMAPPER_PATH,
            keys::MemoryPrivateFilesystem,
            metadata::StratisIdentifiers,
        },
        types::{DevicePath, EncryptionInfo, KeyDescription, SizedKeyMemory},
    },
    stratis::{StratisError, StratisResult},
};

/// Handle for performing operations on an encrypted device.
///
/// This device assumes that its logical, unlocked device path has been activated and
/// is present. This checked in all mechanisms that yield a CryptHandle.
/// * CryptInitializer will ensure that the newly formatted device is activated.
/// * CryptActivationHandle requires the user to activate a device to yield a CryptHandle.
/// * CryptHandle::setup() fails if the device is not active.
///
/// `Clone` is derived for this data structure because `CryptHandle` acquires
/// a next crypt device context for each operation.
#[derive(Debug, Clone)]
pub struct CryptHandle {
    activated_path: DevicePath,
    metadata_handle: CryptMetadataHandle,
}

impl CryptHandle {
    pub(super) fn new(
        physical_path: DevicePath,
        identifiers: StratisIdentifiers,
        encryption_info: EncryptionInfo,
        name: String,
    ) -> StratisResult<CryptHandle> {
        CryptHandle::new_with_metadata_handle(CryptMetadataHandle::new(
            physical_path,
            identifiers,
            encryption_info,
            name,
        ))
    }

    pub(super) fn new_with_metadata_handle(
        metadata_handle: CryptMetadataHandle,
    ) -> StratisResult<CryptHandle> {
        let activated_path = DevicePath::new(
            &once(DEVICEMAPPER_PATH)
                .chain(once(metadata_handle.name()))
                .collect::<PathBuf>(),
        )?;
        Ok(CryptHandle {
            activated_path,
            metadata_handle,
        })
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
    /// keyring. For that, use `CryptActivationHandle::can_unlock()`.
    ///
    /// The checks include:
    /// * is a LUKS2 device
    /// * has a valid Stratis LUKS2 token
    /// * has a token of the proper type for LUKS2 keyring unlocking
    pub fn setup(physical_path: &Path) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => setup_crypt_handle(device, physical_path, None),
            None => Ok(None),
        }
    }

    /// Get the encryption info for this encrypted device.
    pub fn encryption_info(&self) -> &EncryptionInfo {
        self.metadata_handle.encryption_info()
    }

    /// Return the path to the device node of the underlying storage device
    /// for the encrypted device.
    pub fn luks2_device_path(&self) -> &Path {
        self.metadata_handle.luks2_device_path()
    }

    /// Return the path to the device node of the decrypted contents of the encrypted
    /// storage device. In an encrypted pool, this is the path that can be used to read
    /// the Stratis blockdev metatdata.
    pub fn activated_device_path(&self) -> &Path {
        &*self.activated_path
    }

    /// Return the name of the activated devicemapper device.
    pub fn name(&self) -> &str {
        self.metadata_handle.name()
    }

    /// Get the Stratis device identifiers for a given encrypted device.
    pub fn device_identifiers(&self) -> &StratisIdentifiers {
        self.metadata_handle.device_identifiers()
    }

    /// Get the keyslot associated with the given token ID.
    pub fn keyslots(&self, token_id: c_uint) -> StratisResult<Option<Vec<c_uint>>> {
        get_keyslot_number(&mut self.acquire_crypt_device()?, token_id)
    }

    /// Get info for the clevis binding.
    pub fn clevis_info(&self) -> StratisResult<Option<(String, Value)>> {
        clevis_info_from_metadata(&mut self.acquire_crypt_device()?)
    }

    /// Bind the given device using clevis.
    pub fn clevis_bind(&mut self, pin: &str, json: &Value) -> StratisResult<()> {
        let mut json_owned = json.clone();
        let yes = interpret_clevis_config(pin, &mut json_owned)?;

        let key_desc = self
            .metadata_handle
            .encryption_info
            .key_description()
            .ok_or_else(|| {
                StratisError::Msg(
                    "Clevis binding requires a registered key description for the device \
                    but none was found"
                        .to_string(),
                )
            })?;
        let memfs = MemoryPrivateFilesystem::new()?;
        memfs.key_op(key_desc, |keyfile_path| {
            clevis_luks_bind(
                self.luks2_device_path(),
                keyfile_path,
                CLEVIS_LUKS_TOKEN_ID,
                pin,
                &json_owned,
                yes,
            )
        })?;
        self.metadata_handle.encryption_info = self
            .metadata_handle
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
        if self
            .metadata_handle
            .encryption_info
            .key_description()
            .is_none()
        {
            return Err(StratisError::Msg(
                "No kernel keyring binding found; removing the Clevis binding \
                would remove the ability to open this device; aborting"
                    .to_string(),
            ));
        }

        let keyslots = self.keyslots(CLEVIS_LUKS_TOKEN_ID)?.ok_or_else(|| {
            StratisError::Msg(format!(
                "Token slot {} appears to be empty; could not determine keyslots",
                CLEVIS_LUKS_TOKEN_ID,
            ))
        })?;
        for keyslot in keyslots {
            log_on_failure!(
                clevis_luks_unbind(self.luks2_device_path(), keyslot),
                "Failed to unbind device {} from Clevis",
                self.luks2_device_path().display()
            );
        }
        self.metadata_handle.encryption_info = self
            .metadata_handle
            .encryption_info
            .clone()
            .unset_clevis_info();
        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    ///
    /// This method needs to re-read the cached Clevis information because
    /// the config may change specifically in the case where a new thumbprint
    /// is provided if Tang keys are rotated.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        if self.metadata_handle.encryption_info.clevis_info().is_none() {
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
        self.metadata_handle.encryption_info = self
            .metadata_handle
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

        self.metadata_handle.encryption_info = self
            .metadata_handle
            .encryption_info
            .clone()
            .set_key_desc(key_desc.clone());
        Ok(())
    }

    /// Add a keyring binding to the underlying LUKS2 volume.
    pub fn unbind_keyring(&mut self) -> StratisResult<()> {
        if self.metadata_handle.encryption_info.clevis_info().is_none() {
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

        self.metadata_handle.encryption_info = self
            .metadata_handle
            .encryption_info
            .clone()
            .unset_key_desc();

        Ok(())
    }

    /// Change the key description and passphrase that a device is bound to
    pub fn rebind_keyring(&mut self, new_key_desc: &KeyDescription) -> StratisResult<()> {
        let mut device = self.acquire_crypt_device()?;

        let old_key_description = self.metadata_handle.encryption_info
            .key_description()
            .ok_or_else(|| {
                StratisError::Msg("Cannot change passphrase because this device is not bound to a passphrase in the kernel keyring".to_string())
            })?;
        add_keyring_keyslot(
            &mut device,
            new_key_desc,
            Some(Either::Right(old_key_description)),
        )?;
        self.metadata_handle.encryption_info = self
            .metadata_handle
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
                    "Token slot {} is occupied but does not appear to be a Clevis \
                        token; aborting",
                    CLEVIS_LUKS_TOKEN_ID,
                ))
            })?;
        clevis_decrypt(&jwe).map(Some)
    }

    /// Deactivate the device referenced by the current device handle.
    pub fn deactivate(&self) -> StratisResult<()> {
        super::shared::ensure_inactive(&mut self.acquire_crypt_device()?, self.name())
    }

    /// Wipe all LUKS2 metadata on the device safely using libcryptsetup.
    pub fn wipe(&self) -> StratisResult<()> {
        ensure_wiped(
            &mut self.acquire_crypt_device()?,
            self.luks2_device_path(),
            self.name(),
        )
    }

    /// Get the size of the logical device built on the underlying encrypted physical
    /// device. `devicemapper` will return the size in terms of number of sectors.
    pub fn logical_device_size(&self) -> StratisResult<Sectors> {
        let name = self.name().to_owned();
        let active_device = log_on_failure!(
            self.acquire_crypt_device()?
                .runtime_handle(&name)
                .get_active_device(),
            "Failed to get device size for encrypted logical device"
        );
        Ok(Sectors(active_device.size))
    }
}
