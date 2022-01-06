// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filesystem;
mod mdv;
mod thinids;
#[allow(clippy::module_inception)]
mod thinpool;

#[cfg(test)]
pub use self::thinpool::ThinPoolStatusDigest;
pub use self::{
    filesystem::StratFilesystem,
    thinpool::{ThinPool, ThinPoolSizeParams, ThinPoolState, DATA_BLOCK_SIZE},
};
