// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use devicemapper::{IEC, SECTOR_SIZE};

pub use self::engine::BlockDev;
pub use self::engine::Engine;
pub use self::engine::Filesystem;
pub use self::engine::Pool;

pub use self::sim_engine::SimEngine;
pub use self::strat_engine::StratEngine;

pub use self::types::BlockDevState;
pub use self::types::BlockDevTier;
pub use self::types::DevUuid;
pub use self::types::FilesystemUuid;
pub use self::types::Name;
pub use self::types::PoolUuid;
pub use self::types::Redundancy;
pub use self::types::RenameAction;

#[macro_use]
mod macros;

#[allow(module_inception)]
mod engine;
mod sim_engine;
mod strat_engine;
mod structures;
mod types;
