// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use nix::unistd::{pipe, write};

use crate::{
    engine::{KeyDescription, PoolUuid},
    jsonrpc::client::utils::to_suffix_repr,
    print_table,
    stratis::{StratisError, StratisResult},
};

// stratis-min pool create
pub fn pool_create(
    name: String,
    blockdevs: Vec<PathBuf>,
    key_desc: Option<KeyDescription>,
) -> StratisResult<()> {
    do_request_standard!(PoolCreate, name, blockdevs, key_desc)
}

// stratis-min pool unlock
pub fn pool_unlock(uuid: Option<PoolUuid>, prompt: bool) -> StratisResult<()> {
    if prompt {
        do_request_standard!(PoolUnlock, uuid; {
            let password =
                rpassword::prompt_password_stdout("Enter passphrase followed by return:")?;
            let (read_end, write_end) = pipe()?;
            write(write_end, password.as_bytes())?;
            read_end
        })
    } else {
        do_request_standard!(PoolUnlock, uuid)
    }
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

fn size_string(sizes: Vec<(u64, Option<u64>)>) -> Vec<String> {
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
    let (names, sizes, properties) = do_request!(PoolList);
    let physical_col = size_string(sizes);
    let properties_col = properties_string(properties);
    print_table!(
        "Name", names, "<";
        "Total Physical", physical_col, ">";
        "Properties", properties_col, ">"
    );

    Ok(())
}

// stratis-min is-encrypted
pub fn pool_is_encrypted(uuid: PoolUuid) -> StratisResult<bool> {
    let (is_encrypted, rc, rs) = do_request!(PoolIsEncrypted, uuid);
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        Ok(is_encrypted)
    }
}

// stratis-min is-locked
pub fn pool_is_locked(uuid: PoolUuid) -> StratisResult<bool> {
    let (is_locked, rc, rs) = do_request!(PoolIsLocked, uuid);
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        Ok(is_locked)
    }
}
