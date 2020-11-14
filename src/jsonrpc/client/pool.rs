// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{io::stdin, os::unix::io::AsRawFd, path::PathBuf};

use crate::{
    engine::{KeyDescription, PoolUuid},
    jsonrpc::interface::{StratisParamType, StratisParams},
    print_table,
    stratis::{StratisError, StratisResult},
};

const SUFFIXES: &[(u64, &str)] = &[
    (60, "EiB"),
    (50, "PiB"),
    (40, "TiB"),
    (30, "GiB"),
    (20, "MiB"),
    (10, "KiB"),
    (1, "B"),
];

// stratis-min pool create
pub fn pool_create(
    name: String,
    blockdevs: Vec<PathBuf>,
    key_desc: Option<KeyDescription>,
) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolCreate(name, blockdevs, key_desc),
            fd_opt: None,
        },
        PoolCreate
    )
}

// stratis-min pool unlock
pub fn pool_unlock(uuid: PoolUuid, prompt: bool) -> StratisResult<()> {
    if prompt {
        println!("Enter passphrase followed by return:");
    }
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolUnlock(uuid, prompt),
            fd_opt: if prompt {
                Some(stdin().as_raw_fd())
            } else {
                None
            }
        },
        PoolUnlock
    )
}

// stratis-min pool init-cache
pub fn pool_init_cache(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolInitCache(name, paths),
            fd_opt: None,
        },
        PoolInitCache
    )
}

// stratis-min pool init-cache
pub fn pool_rename(name: String, new_name: String) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolRename(name, new_name),
            fd_opt: None,
        },
        PoolRename
    )
}

// stratis-min pool add-data
pub fn pool_add_data(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolAddData(name, paths),
            fd_opt: None,
        },
        PoolAddData
    )
}

// stratis-min pool add-cache
pub fn pool_add_cache(name: String, paths: Vec<PathBuf>) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolAddCache(name, paths),
            fd_opt: None,
        },
        PoolAddCache
    )
}

// stratis-min pool destroy
pub fn pool_destroy(name: String) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::PoolDestroy(name),
            fd_opt: None,
        },
        PoolDestroy
    )
}

#[allow(clippy::cast_precision_loss)]
fn to_suffix_repr(size: u64) -> String {
    SUFFIXES.iter().fold(String::new(), |acc, (div, suffix)| {
        let div_shifted = 1 << div;
        if acc.is_empty() && size / div_shifted >= 1 {
            format!(
                "{:.2} {}",
                (size / div_shifted) as f64 + ((size % div_shifted) as f64 / div_shifted as f64),
                suffix
            )
        } else {
            acc
        }
    })
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
    let (names, sizes, properties) = do_request!(
        StratisParams {
            type_: StratisParamType::PoolList,
            fd_opt: None,
        },
        PoolList
    );
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
    let (is_encrypted, rc, rs) = do_request!(
        StratisParams {
            type_: StratisParamType::PoolIsEncrypted(uuid),
            fd_opt: None,
        },
        PoolIsEncrypted
    );
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        Ok(is_encrypted)
    }
}
