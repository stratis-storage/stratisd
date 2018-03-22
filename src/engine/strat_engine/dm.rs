// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.


use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::{Once, ONCE_INIT};

use devicemapper::DM;

use super::super::engine::Eventable;
use super::super::errors::EngineResult;

static INIT: Once = ONCE_INIT;
static mut DM_CONTEXT: Option<DM> = None;

pub fn get_dm_context() -> &'static DM {
    unsafe {
        INIT.call_once(|| DM_CONTEXT = Some(DM::new().unwrap()));
        match DM_CONTEXT {
            Some(ref context) => context,
            _ => panic!("DM_CONTEXT.is_some()"),
        }
    }
}


impl Eventable for DM {
    /// Get file we'd like to have monitored for activity
    fn get_pollable_fd(&self) -> RawFd {
        self.file().as_raw_fd()
    }

    fn clear_event(&self) -> EngineResult<()> {
        self.arm_poll()?;
        Ok(())
    }
}
