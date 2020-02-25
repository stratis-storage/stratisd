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
mod setup;
mod shared;
mod udev;

pub use self::{
    backstore::Backstore,
    blockdev::StratBlockDev,
    identify::{find_all, identify_possibly_encrypted_block_device, Identification, StratisInfo},
    metadata::MDADataSize,
    setup::get_metadata,
};

#[cfg(test)]
pub use self::{crypt::STRATIS_KEY_SIZE, identify::find_all_block_devices_with_stratis_signatures};
