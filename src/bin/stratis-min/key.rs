// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    io::{self, Read},
    os::unix::io::AsRawFd,
};

use libcryptsetup_rs::SafeMemHandle;
use libstratis::{
    engine::{
        DeleteAction, KeyActions, MappingCreateAction, SizedKeyMemory, StratKeyActions,
        MAX_STRATIS_PASS_SIZE,
    },
    stratis::{StratisError, StratisResult},
};

use crate::print_table;

/// This method sets a key in the kernel keyring. It accepts an optional keyfile path
/// and if this is not provided, the user is prompted for a passphrase. When `no_tty`
/// is true, this command assumes that no TTY is available for setting terminal
/// settings and settings such as `NOECHO` are not set. This option should be
/// used carefully as it will cause the password to be echoed on the screen if
/// invoked interactively.
// stratis-min key set
pub fn key_set(key_desc: &str, keyfile_path: Option<&str>) -> StratisResult<()> {
    let ret = match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            StratKeyActions.set(key_desc, file.as_raw_fd(), None)?
        }
        None => {
            let mut stdin = io::stdin();
            let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
            println!("Enter desired key data followed by the return key:");
            let i = stdin.read(mem.as_mut())?;
            StratKeyActions.set_no_fd(key_desc, SizedKeyMemory::new(mem, i))?
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

// stratis-min key unset
pub fn key_unset(key_desc: &str) -> StratisResult<()> {
    match StratKeyActions.unset(key_desc)? {
        DeleteAction::Deleted(()) => Ok(()),
        DeleteAction::Identity => Err(StratisError::Error(format!(
            "Key with key description {} does not exist.",
            key_desc
        ))),
    }
}

// stratis-min key [list]
pub fn key_list() -> StratisResult<()> {
    print_table!(
        "Key Description",
        StratKeyActions
            .list()?
            .into_iter()
            .map(|kd| kd.as_application_str().to_string())
            .collect::<Vec<_>>(),
        "<"
    );
    Ok(())
}
