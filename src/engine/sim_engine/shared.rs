// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{
    engine::types::{EncryptionInfo, InputEncryptionInfo, UnlockMechanism},
    stratis::{StratisError, StratisResult},
};

use super::keys::SimKeyActions;

pub fn convert_encryption_info(
    encryption_info: Option<&InputEncryptionInfo>,
    key_handler: Option<&SimKeyActions>,
) -> StratisResult<Option<EncryptionInfo>> {
    encryption_info
        .cloned()
        .map(|ei| {
            ei.into_iter().try_fold(
                EncryptionInfo::new(),
                |mut info, (token_slot, unlock_mechanism)| {
                    let ts = match token_slot {
                        Some(t) => t,
                        None => info.free_token_slot(),
                    };
                    if let UnlockMechanism::KeyDesc(ref kd) = unlock_mechanism {
                        if let Some(kh) = key_handler {
                            if !kh.contains_key(kd) {
                                return Err(StratisError::Msg(format!(
                                    "Key {} was not found in the keyring",
                                    kd.as_application_str()
                                )));
                            }
                        }
                    }
                    info.add_info(ts, unlock_mechanism)?;
                    Ok(info)
                },
            )
        })
        .transpose()
}
