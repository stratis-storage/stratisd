// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{os::fd::AsRawFd, path::PathBuf};

use nix::unistd::{pipe, write};

use serde_json::Value;

use crate::{
    engine::{
        InputEncryptionInfo, KeyDescription, OptionalTokenSlotInput, PoolIdentifier, PoolUuid,
        TokenUnlockMethod,
    },
    jsonrpc::client::utils::{prompt_password, to_suffix_repr},
    print_table,
    stratis::{StratisError, StratisResult},
};

// stratis-min pool create
pub fn pool_create(
    name: String,
    blockdevs: Vec<PathBuf>,
    enc_info: Option<InputEncryptionInfo>,
) -> StratisResult<()> {
    do_request_standard!(PoolCreate, name, blockdevs, enc_info)
}

// stratis-min pool start
pub fn pool_start(
    id: PoolIdentifier<PoolUuid>,
    unlock_method: TokenUnlockMethod,
    prompt: bool,
) -> StratisResult<()> {
    if prompt {
        let password = prompt_password()?
            .ok_or_else(|| StratisError::Msg("Password provided was empty".to_string()))?;

        let (read_end, write_end) = pipe()?;
        write(write_end, password.as_bytes())?;
        do_request_standard!(PoolStart, id, unlock_method; {
            read_end.as_raw_fd()
        })
    } else {
        do_request_standard!(PoolStart, id, unlock_method)
    }
}

// stratis-min pool stop
pub fn pool_stop(id: PoolIdentifier<PoolUuid>) -> StratisResult<()> {
    do_request_standard!(PoolStop, id)
}

// stratis-min pool init-cache
pub fn pool_init_cache(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(PoolInitCache, name, paths)
}

// stratis-min pool init-cache
pub fn pool_rename(name: String, new_name: String) -> StratisResult<()> {
    do_request_standard!(PoolRename, name, new_name)
}

// stratis-min pool add-data
pub fn pool_add_data(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(PoolAddData, name, paths)
}

// stratis-min pool add-cache
pub fn pool_add_cache(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(PoolAddCache, name, paths)
}

// stratis-min pool destroy
pub fn pool_destroy(name: String) -> StratisResult<()> {
    do_request_standard!(PoolDestroy, name)
}

fn size_string(sizes: Vec<(u128, Option<u128>)>) -> Vec<String> {
    sizes
        .into_iter()
        .map(|(size, used)| {
            let free = used.map(|u| size - u);
            format!(
                "{} / {} / {}",
                to_suffix_repr(size),
                match used {
                    Some(u) => to_suffix_repr(u),
                    None => "FAILURE".to_string(),
                },
                match free {
                    Some(f) => to_suffix_repr(f),
                    None => "FAILURE".to_string(),
                },
            )
        })
        .collect()
}

fn properties_string(properties: Vec<(bool, bool)>) -> Vec<String> {
    properties
        .into_iter()
        .map(|(has_cache, is_encrypted)| {
            let ca = if has_cache { " Ca" } else { "~Ca" };
            let cr = if is_encrypted { " Cr" } else { "~Cr" };
            [ca, cr].join(",")
        })
        .collect()
}

// stratis-min pool [list]
pub fn pool_list() -> StratisResult<()> {
    let (names, sizes, properties, uuids) = do_request!(PoolList);
    let physical_col = size_string(sizes);
    let properties_col = properties_string(properties);
    print_table!(
        "Name", names, "<";
        "Total Physical", physical_col, ">";
        "Properties", properties_col, ">";
        "UUID", uuids.iter().map(|u| u.to_string()).collect::<Vec<_>>(), ">"
    );

    Ok(())
}

// stratis-min pool is-encrypted
pub fn pool_is_encrypted(id: PoolIdentifier<PoolUuid>) -> StratisResult<bool> {
    let (is_encrypted, rc, rs) = do_request!(PoolIsEncrypted, id);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(is_encrypted)
    }
}

// stratis-min pool is-stopped
pub fn pool_is_stopped(id: PoolIdentifier<PoolUuid>) -> StratisResult<bool> {
    let (is_stopped, rc, rs) = do_request!(PoolIsStopped, id);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(is_stopped)
    }
}

// stratis-min pool has-passphrase
pub fn pool_has_passphrase(id: PoolIdentifier<PoolUuid>) -> StratisResult<bool> {
    let (has_passphrase, rc, rs) = do_request!(PoolHasPassphrase, id);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(has_passphrase)
    }
}

pub fn pool_bind_keyring(
    id: PoolIdentifier<PoolUuid>,
    token_slot: OptionalTokenSlotInput,
    key_desc: KeyDescription,
) -> StratisResult<()> {
    do_request_standard!(PoolBindKeyring, id, token_slot, key_desc)
}

pub fn pool_bind_clevis(
    id: PoolIdentifier<PoolUuid>,
    token_slot: OptionalTokenSlotInput,
    pin: String,
    clevis_info: Value,
) -> StratisResult<()> {
    do_request_standard!(PoolBindClevis, id, token_slot, pin, clevis_info)
}

pub fn pool_unbind_keyring(
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<()> {
    do_request_standard!(PoolUnbindKeyring, id, token_slot)
}

pub fn pool_unbind_clevis(
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<()> {
    do_request_standard!(PoolUnbindClevis, id, token_slot)
}

pub fn pool_rebind_keyring(
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
    key_desc: KeyDescription,
) -> StratisResult<()> {
    do_request_standard!(PoolRebindKeyring, id, token_slot, key_desc)
}

pub fn pool_rebind_clevis(
    id: PoolIdentifier<PoolUuid>,
    token_slot: Option<u32>,
) -> StratisResult<()> {
    do_request_standard!(PoolRebindClevis, id, token_slot)
}
