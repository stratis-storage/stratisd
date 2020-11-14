// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::File, io::stdin, os::unix::io::AsRawFd};

use crate::{
    engine::KeyDescription,
    jsonrpc::interface::{StratisParamType, StratisParams},
    print_table,
    stratis::{StratisError, StratisResult},
};

pub fn key_set(key_desc: KeyDescription, keyfile_path: Option<&str>) -> StratisResult<()> {
    let (changed, rc, rs) = match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            do_request!(
                StratisParams {
                    type_: StratisParamType::KeySet(key_desc),
                    fd_opt: Some(file.as_raw_fd()),
                },
                KeySet
            )
        }
        None => {
            println!("Enter passphrase followed by return:");
            do_request!(
                StratisParams {
                    type_: StratisParamType::KeySet(key_desc),
                    fd_opt: Some(stdin().as_raw_fd()),
                },
                KeySet
            )
        }
    };
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else if changed.is_none() {
        Err(StratisError::Error(
            "The requested action had no effect".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub fn key_unset(key_desc: KeyDescription) -> StratisResult<()> {
    do_request_standard!(
        StratisParams {
            type_: StratisParamType::KeyUnset(key_desc),
            fd_opt: None,
        },
        KeyUnset
    )
}

pub fn key_list() -> StratisResult<()> {
    let (info, rc, rs): (Vec<KeyDescription>, u16, String) = do_request!(
        StratisParams {
            type_: StratisParamType::KeyList,
            fd_opt: None,
        },
        KeyList
    );
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        let key_desc_strings = info
            .into_iter()
            .map(|kd| kd.as_application_str().to_string())
            .collect::<Vec<_>>();
        print_table!("Key Description", key_desc_strings, "<");
        Ok(())
    }
}
