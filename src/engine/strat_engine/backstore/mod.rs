// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(clippy::module_inception)]
mod backstore;
mod blockdev;
mod blockdevmgr;
mod cache_tier;
mod data_tier;
mod device;
mod identify;
mod metadata;
mod range_alloc;
mod setup;
mod shared;

pub use self::{
    backstore::Backstore,
    blockdev::StratBlockDev,
    device::DevOwnership,
    identify::{find_all, identify_block_device},
    metadata::MDADataSize,
    setup::get_metadata,
};
