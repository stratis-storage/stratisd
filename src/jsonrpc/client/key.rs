// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::File, os::unix::io::AsRawFd};

use nix::unistd::{pipe, write};

use crate::{
    engine::KeyDescription,
    jsonrpc::client::utils::prompt_password,
    print_table,
    stratis::{StratisError, StratisResult},
};

pub fn key_set(key_desc: KeyDescription, keyfile_path: Option<&str>) -> StratisResult<()> {
    let (changed, rc, rs) = match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            do_request!(KeySet, key_desc; file.as_raw_fd())
        }
        None => {
            let password = prompt_password()?
                .ok_or_else(|| StratisError::Msg("Password provided was empty".to_string()))?;

            let (read_end, write_end) = pipe()?;
            write(write_end, password.as_bytes())?;
            do_request!(KeySet, key_desc; read_end.as_raw_fd())
        }
    };
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else if changed.is_none() {
        Err(StratisError::Msg(
            "The requested action had no effect".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub fn key_unset(key_desc: KeyDescription) -> StratisResult<()> {
    do_request_standard!(KeyUnset, key_desc)
}

pub fn key_list() -> StratisResult<()> {
    let (info, rc, rs): (Vec<KeyDescription>, u16, String) = do_request!(KeyList);
    if rc != 0 {
        Err(StratisError::Msg(rs))
    } else {
        let key_desc_strings = info
            .into_iter()
            .map(|kd| kd.as_application_str().to_string())
            .collect::<Vec<_>>();
        print_table!("Key Description", key_desc_strings, "<");
        Ok(())
    }
}
