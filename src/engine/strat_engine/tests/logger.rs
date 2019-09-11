// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Once;

static LOGGER_INIT: Once = Once::new();

/// Initialize the logger once.  More than one init() attempt returns
/// errors.
pub fn init_logger() {
    LOGGER_INIT.call_once(env_logger::init);
}
