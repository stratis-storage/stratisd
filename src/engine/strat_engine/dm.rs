// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.

use std::{
    os::unix::io::{AsRawFd, RawFd},
    sync::Once,
};

use devicemapper::{DmResult, DM};

use crate::{
    engine::engine::Eventable,
    stratis::{ErrorEnum, StratisError, StratisResult},
};

static INIT: Once = Once::new();
static mut DM_CONTEXT: Option<DmResult<DM>> = None;

pub fn get_dm_init() -> StratisResult<&'static DM> {
    unsafe {
        INIT.call_once(|| DM_CONTEXT = Some(DM::new()));
        match DM_CONTEXT {
            Some(Ok(ref context)) => Ok(context),
            Some(Err(ref e)) => Err(StratisError::Engine(
                ErrorEnum::Error,
                format!("Failed to initialize DM context: {}", e),
            )),
            _ => panic!("DM_CONTEXT.is_some()"),
        }
    }
}

pub fn get_dm() -> &'static DM {
    get_dm_init().expect(
        "the engine has already called get_dm_init() and exited if get_dm_init() returned an error",
    )
}

impl Eventable for DM {
    /// Get file we'd like to have monitored for activity
    fn get_pollable_fd(&self) -> RawFd {
        self.file().as_raw_fd()
    }

    fn clear_event(&self) -> StratisResult<()> {
        self.arm_poll()?;
        Ok(())
    }
}
