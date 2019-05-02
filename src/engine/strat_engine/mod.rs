// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod backstore;
#[cfg(test)]
mod cleanup;
pub mod cmd;
mod device;
mod dm;
mod engine;
mod names;
mod pool;
mod serde_structs;
mod thinpool;

pub use self::engine::StratEngine;

#[cfg(test)]
mod tests;
