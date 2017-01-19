// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod blockdev;
pub mod engine;
pub mod metadata;
pub mod filesystem;
pub mod pool;

mod serde_structs {
    include!(concat!(env!("OUT_DIR"), "/serde_structs.rs"));
}

pub use self::engine::StratEngine;
pub use self::pool::StratPool;
pub use self::serde_structs::BlockDevSave;
