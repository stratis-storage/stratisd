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
mod range_alloc;
mod shared;

pub use self::{
    backstore::Backstore,
    blockdev::{StratBlockDev, UnderlyingDevice},
    crypt::{
        crypt_metadata_size, CryptActivationHandle, CryptHandle, CryptMetadataHandle,
        CLEVIS_TANG_TRUST_URL,
    },
};

#[cfg(test)]
pub use self::devices::{initialize_devices, process_and_verify_devices};
