// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod backstore;
mod cmd;
mod crypt;
mod device;
mod devlinks;
mod dm;
mod engine;
mod keys;
mod liminal;
mod metadata;
mod names;
mod ns;
mod pool;
mod serde_structs;
mod shared;
mod thinpool;
mod types;
mod udev;
mod writing;

pub use self::{
    crypt::{
        crypt_metadata_size, handle::v1::CryptHandle, set_up_crypt_logging, CLEVIS_TANG_TRUST_URL,
    },
    dm::{get_dm, get_dm_init},
    engine::StratEngine,
    keys::StratKeyActions,
    metadata::{StaticHeader, StaticHeaderResult, BDA},
    ns::unshare_mount_namespace,
    thinpool::ThinPoolSizeParams,
};

#[cfg(test)]
mod tests;
