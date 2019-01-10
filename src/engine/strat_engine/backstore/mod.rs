// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(clippy::module_inception)]
mod backstore;
mod blockdev;
mod blockdevmgr;
mod cache_tier;
mod cleanup;
mod data_tier;
pub mod device;
mod metadata;
mod range_alloc;
mod setup;
mod udev;

pub use self::{
    backstore::Backstore,
    blockdev::StratBlockDev,
    device::{blkdev_size, is_stratis_device},
    metadata::MDADataSize,
    setup::{find_all, get_metadata},
};
