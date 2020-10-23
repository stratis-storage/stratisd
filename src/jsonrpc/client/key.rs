// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::File, os::unix::io::AsRawFd};

use nix::unistd::{pipe, write};

use crate::{
    engine::KeyDescription,
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
            let password =
                rpassword::prompt_password_stdout("Enter passphrase followed by return:")?;
            let (read_end, write_end) = pipe()?;
            write(write_end, password.as_bytes())?;
            do_request!(KeySet, key_desc; read_end)
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
    do_request_standard!(KeyUnset, key_desc)
}

pub fn key_list() -> StratisResult<()> {
    let (info, rc, rs): (Vec<KeyDescription>, u16, String) = do_request!(KeyList);
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
