// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
mod utils;

mod filesystem;
mod key;
mod pool;
mod report;
#[allow(clippy::module_inception)]
mod server;

pub use server::run_server;
