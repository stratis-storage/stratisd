// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.

use std::sync::Once;

use devicemapper::{DmResult, DM};

use crate::stratis::{StratisError, StratisResult};

static INIT: Once = Once::new();
static mut DM_CONTEXT: Option<DmResult<DM>> = None;

pub fn get_dm_init() -> StratisResult<&'static DM> {
    unsafe {
        INIT.call_once(|| DM_CONTEXT = Some(DM::new()));
        match &DM_CONTEXT {
            Some(Ok(ref context)) => Ok(context),
            Some(Err(e)) => Err(StratisError::Chained(
                "Failed to initialize DM context".to_string(),
                // FIXME: Cannot clone DMError
                Box::new(StratisError::Msg(e.to_string())),
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
