// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    io::stdin,
    os::unix::{io::AsRawFd, net::UnixStream},
};

use crate::{
    do_request,
    jsonrpc::{utils::send_fd_to_sock, Stratis, SOCKFD_ADDR},
    print_table,
    stratis::{StratisError, StratisResult},
};

pub fn key_set(key_desc: &str, keyfile_path: Option<&str>, no_tty: bool) -> StratisResult<()> {
    let stream = UnixStream::connect(SOCKFD_ADDR)?;
    match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            send_fd_to_sock(stream.as_raw_fd(), file.as_raw_fd())?;
        }
        None => {
            send_fd_to_sock(stream.as_raw_fd(), stdin().as_raw_fd())?;
            println!("Enter passphrase followed by return:");
        }
    };
    let (changed, rc, rs): (Option<bool>, u16, String) = do_request!(
        Stratis::key_set,
        key_desc,
        if keyfile_path.is_none() {
            Some(!no_tty)
        } else {
            None
        }
    );
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

pub fn key_unset(key_desc: &str) -> StratisResult<()> {
    let (deleted, rc, rs): (bool, u16, String) = do_request!(Stratis::key_unset, key_desc);
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else if !deleted {
        Err(StratisError::Error(
            "The requested action had no effect".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub fn key_list() -> StratisResult<()> {
    let (info, rc, rs): (Vec<String>, u16, String) = do_request!(Stratis::key_list);
    if rc != 0 {
        Err(StratisError::Error(rs))
    } else {
        print_table!("Key Description", info, "<");
        Ok(())
    }
}
