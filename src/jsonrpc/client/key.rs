// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::File;

use crate::{
    do_request, do_request_standard,
    engine::KeyDescription,
    jsonrpc::Stratis,
    print_table,
    stratis::{StratisError, StratisResult},
};

pub fn key_set(key_desc: KeyDescription, keyfile_path: Option<&str>) -> StratisResult<()> {
    match keyfile_path {
        Some(kp) => {
            let _file = File::open(kp)?;
            //send_fd_to_sock(stream.as_raw_fd(), file.as_raw_fd())?;
        }
        None => {
            //send_fd_to_sock(stream.as_raw_fd(), stdin().as_raw_fd())?;
            println!("Enter passphrase followed by return:");
        }
    };
    let (changed, rc, rs): (Option<bool>, u16, String) = do_request!(Stratis::key_set, key_desc);
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
    do_request_standard!(Stratis::key_unset, key_desc)
}

pub fn key_list() -> StratisResult<()> {
    let (info, rc, rs): (Vec<KeyDescription>, u16, String) = do_request!(Stratis::key_list);
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
