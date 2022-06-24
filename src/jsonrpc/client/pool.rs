// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use nix::unistd::{pipe, write};

use crate::{
    engine::{EncryptionInfo, PoolUuid, UnlockMethod},
    jsonrpc::client::utils::to_suffix_repr,
    print_table,
    stratis::{StratisError, StratisResult},
};

// stratis-min pool create
pub fn pool_create(
    name: String,
    blockdevs: Vec<PathBuf>,
    enc_info: Option<EncryptionInfo>,
) -> StratisResult<()> {
    do_request_standard!(PoolCreate, name, blockdevs, enc_info)
}

// stratis-min pool start
pub fn pool_start(
    uuid: PoolUuid,
    unlock_method: Option<UnlockMethod>,
    prompt: bool,
) -> StratisResult<()> {
    if prompt {
        let password = rpassword::prompt_password_stdout("Enter passphrase followed by return: ")?;
        if password.is_empty() {
            return Ok(());
        }
        do_request_standard!(PoolStart, uuid, unlock_method; {
            let (read_end, write_end) = pipe()?;
            write(write_end, password.as_bytes())?;
            read_end
        })
    } else {
        do_request_standard!(PoolStart, uuid, unlock_method)
    }
}

// stratis-min pool stop
pub fn pool_stop(uuid: PoolUuid) -> StratisResult<()> {
    do_request_standard!(PoolStop, uuid)
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
            vec![ca, cr].join(",")
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
pub fn pool_is_encrypted(uuid: PoolUuid) -> StratisResult<bool> {
    let (is_encrypted, rc, rs) = do_request!(PoolIsEncrypted, uuid);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(is_encrypted)
    }
}

// stratis-min pool is-stopped
pub fn pool_is_stopped(uuid: PoolUuid) -> StratisResult<bool> {
    let (is_stopped, rc, rs) = do_request!(PoolIsStopped, uuid);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(is_stopped)
    }
}

// stratis-min pool is-bound
pub fn pool_is_bound(uuid: PoolUuid) -> StratisResult<bool> {
    let (is_bound, rc, rs) = do_request!(PoolIsBound, uuid);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(is_bound)
    }
}

// stratis-min pool has-passphrase
pub fn pool_has_passphrase(uuid: PoolUuid) -> StratisResult<bool> {
    let (has_passphrase, rc, rs) = do_request!(PoolHasPassphrase, uuid);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(has_passphrase)
    }
}

// stratis-min pool clevis-pin
pub fn pool_clevis_pin(uuid: PoolUuid) -> StratisResult<String> {
    let (clevis_pin, rc, rs) = do_request!(PoolClevisPin, uuid);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        Ok(clevis_pin.unwrap_or_else(|| "None".to_string()))
    }
}
