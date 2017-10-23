// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod blockdev;
mod blockdevmgr;
mod cleanup;
mod device;
mod dmdevice;
mod engine;
mod metadata;
mod mdv;
mod filesystem;
mod pool;
mod serde_structs;
mod setup;
mod range_alloc;
mod thinpool;
pub mod util;

pub use self::engine::StratEngine;

#[cfg(test)]
mod tests;
