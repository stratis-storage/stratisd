// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use self::consts::IEC;

pub use self::engine::BlockDev;
pub use self::engine::Engine;
pub use self::engine::Filesystem;
pub use self::engine::Pool;

pub use self::errors::EngineError;
pub use self::errors::EngineResult;
pub use self::errors::ErrorEnum;

pub use self::sim_engine::SimEngine;
pub use self::strat_engine::StratEngine;

pub use self::types::DevUuid;
pub use self::types::FilesystemUuid;
pub use self::types::PoolUuid;
pub use self::types::Redundancy;
pub use self::types::RenameAction;

#[macro_use]
mod macros;

// strat_engine is public so that integration tests can access its internals.
pub mod strat_engine;

mod consts;
#[allow(module_inception)]
pub mod engine;
mod errors;
mod sim_engine;
mod structures;
pub mod types;
