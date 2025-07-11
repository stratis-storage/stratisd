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
mod udev;
mod writing;

#[cfg(feature = "extras")]
pub use self::{
    backstore::ProcessedPathInfos,
    pool::{inspection as pool_inspection, v1::StratPool},
};

pub use self::{
    backstore::integrity_meta_space,
    crypt::{
        register_clevis_token, set_up_crypt_logging, CLEVIS_LUKS_TOKEN_ID, CLEVIS_TANG_TRUST_URL,
        DEFAULT_CRYPT_DATA_OFFSET_V2, LUKS2_TOKEN_ID,
    },
    dm::{get_dm, get_dm_init},
    engine::StratEngine,
    keys::{create_process_keyring, StratKeyActions},
    metadata::{StaticHeader, StaticHeaderResult, BDA},
    ns::unshare_mount_namespace,
    thinpool::ThinPoolSizeParams,
};

#[cfg(test)]
mod tests;
