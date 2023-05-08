// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
pub mod utils;

#[allow(clippy::module_inception)]
mod client;
pub mod filesystem;
pub mod key;
pub mod pool;
pub mod report;

pub use self::client::StratisClient;
