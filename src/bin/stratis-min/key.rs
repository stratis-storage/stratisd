// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::File, io, os::unix::io::AsRawFd};

use libstratis::{
    engine::{DeleteAction, KeyActions, MappingCreateAction, StratKeyActions},
    stratis::{StratisError, StratisResult},
};

pub fn key_set(key_desc: &str, keyfile_path: Option<&str>, no_tty: bool) -> StratisResult<()> {
    let ret = match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            StratKeyActions.set(key_desc, file.as_raw_fd(), None)?
        }
        None => {
            let stdin_fd = io::stdin().as_raw_fd();
            println!("Enter desired key data followed by the return key:");
            StratKeyActions.set(key_desc, stdin_fd, Some(!no_tty))?
        }
    };
    match ret {
        MappingCreateAction::Identity => Err(StratisError::Error(format!(
            "Key with key description {} was unchanged by the set action",
            key_desc
        ))),
        MappingCreateAction::Created(()) => Ok(()),
        MappingCreateAction::ValueChanged(()) => Ok(()),
    }
}

pub fn key_unset(key_desc: &str) -> StratisResult<()> {
    match StratKeyActions.unset(key_desc)? {
        DeleteAction::Deleted(()) => Ok(()),
        DeleteAction::Identity => Err(StratisError::Error(format!(
            "Key with key description {} does not exist.",
            key_desc
        ))),
    }
}

pub fn key_list() -> StratisResult<()> {
    let keys = StratKeyActions.list()?;
    println!("Key description");
    for key in keys.iter() {
        println!("{}", key.as_application_str());
    }
    Ok(())
}
