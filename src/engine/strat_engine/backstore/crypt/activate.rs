// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use libcryptsetup_rs::consts::flags::CryptActivate;

use crate::{
    engine::{
        strat_engine::{
            backstore::crypt::{
                consts::CLEVIS_LUKS_TOKEN_ID,
                handle::CryptHandle,
                shared::{
                    acquire_crypt_device, check_luks2_token, get_keyslot_number,
                    key_desc_from_metadata, setup_crypt_device, setup_crypt_handle,
                },
            },
            cmd::clevis_decrypt,
        },
        types::UnlockMethod,
    },
    stratis::{StratisError, StratisResult},
};

/// Handle for activating a locked encrypted device.
pub struct CryptActivationHandle;

impl CryptActivationHandle {
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
                let token = device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).ok();
                let jwe = token.as_ref().and_then(|t| t.get("jwe"));
                if let Some(jwe) = jwe {
                    let pass = clevis_decrypt(jwe)?;
                    if let Some(keyslot) = get_keyslot_number(&mut device, CLEVIS_LUKS_TOKEN_ID)?
                        .and_then(|k| k.into_iter().next())
                    {
                        log_on_failure!(
                            device.activate_handle().activate_by_passphrase(
                                None,
                                Some(keyslot),
                                pass.as_ref(),
                                CryptActivate::empty(),
                            ),
                            "libcryptsetup reported that the decrypted Clevis passphrase \
                            is unable to open the encrypted device"
                        );
                    } else {
                        return Err(StratisError::Msg(
                            "Clevis JWE was found in the Stratis metadata but was \
                            not associated with any keyslots"
                                .to_string(),
                        ));
                    }
                }
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

    /// Query the device metadata to reconstruct a handle for performing operations
    /// on an existing encrypted device.
    ///
    /// This method will check that the metadata on the given device is
    /// for the LUKS2 format and that the LUKS2 metadata is formatted
    /// properly as a Stratis encrypted device. If it is properly
    /// formatted it will return the device identifiers (pool and device UUIDs).
    ///
    /// NOTE: This method attempts to activate the device and thus returns a CryptHandle
    ///
    /// The checks include:
    /// * is a LUKS2 device
    /// * has a valid Stratis LUKS2 token
    /// * has a token of the proper type for LUKS2 keyring unlocking
    pub fn setup(
        physical_path: &Path,
        unlock_method: UnlockMethod,
    ) -> StratisResult<Option<CryptHandle>> {
        match setup_crypt_device(physical_path)? {
            Some(ref mut device) => setup_crypt_handle(device, physical_path, Some(unlock_method)),
            None => Ok(None),
        }
    }
}
