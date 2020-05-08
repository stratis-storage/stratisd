// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(clippy::module_inception)]
mod backstore;
mod blockdev;
mod blockdevmgr;
mod cache_tier;
mod crypt;
mod data_tier;
mod devices;
mod identify;
mod metadata;
mod range_alloc;
mod shared;
mod udev;

pub use self::{
    backstore::Backstore,
    blockdev::StratBlockDev,
    crypt::CryptHandle,
    identify::{find_all, identify_block_device, StratisInfo},
    metadata::{MDADataSize, StratisIdentifiers, BDA},
};
