// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate env_logger;

use std::sync::{Once, ONCE_INIT};

static LOGGER_INIT: Once = ONCE_INIT;

/// Initialize the logger once.  More than one init() attempt returns
/// errors.
pub fn init_logger() {
    LOGGER_INIT.call_once(|| {
                              env_logger::init()
            .expect("This is the first and only initialization of the logger; it must succeed");
                          });
}
