// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(clippy::module_inception)]
mod backstore;
mod blockdev;
mod blockdevmgr;
mod cache_tier;
mod data_tier;
mod devices;
mod initialize;
mod range_alloc;
mod shared;

pub use self::{backstore::Backstore, blockdev::StratBlockDev};

#[cfg(test)]
pub use self::{devices::process_and_verify_devices, initialize::initialize_devices};
