// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod blockdev;
mod engine;
mod filesystem;
mod metadata;
mod pool;
mod setup;
mod util;
mod consts;

mod serde_structs {
    include!(concat!(env!("OUT_DIR"), "/serde_structs.rs"));
}

pub use self::engine::StratEngine;
pub use self::pool::StratPool;
pub use self::serde_structs::BlockDevSave;
