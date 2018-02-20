// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod backstore;
mod cleanup;
mod device;
mod devlinks;
mod dmnames;
mod engine;
mod pool;
mod serde_structs;
mod thinpool;

pub use self::engine::StratEngine;

#[cfg(test)]
mod tests;
