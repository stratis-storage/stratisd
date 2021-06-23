// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use self::{
    errors::{StratisError, StratisResult},
    run::run,
    stratis::VERSION,
};

mod dm;
mod errors;
mod ipc_support;
mod run;
#[allow(clippy::module_inception)]
mod stratis;
mod udev_monitor;
