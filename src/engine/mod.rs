// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub use self::{
    engine::{BlockDev, Engine, Filesystem, KeyActions, Pool, Report},
    shared::{total_allocated, total_used},
    sim_engine::SimEngine,
    strat_engine::{
        crypt_metadata_size, get_dm, get_dm_init, StaticHeader, StaticHeaderResult, StratEngine,
        StratKeyActions, ThinPoolSizeParams, BDA, CLEVIS_TANG_TRUST_URL,
    },
    structures::{ExclusiveGuard, SharedGuard, Table},
    types::{
        ActionAvailability, BlockDevTier, ClevisInfo, CreateAction, DeleteAction, DevUuid, Diff,
        EncryptionInfo, EngineAction, FilesystemUuid, KeyDescription, LockKey, Lockable,
        LockedPoolInfo, MappingCreateAction, MappingDeleteAction, MaybeInconsistent, Name,
        PoolDiff, PoolEncryptionInfo, PoolUuid, RenameAction, ReportType, SetCreateAction,
        SetDeleteAction, StratFilesystemDiff, StratPoolDiff, StratisUuid, ThinPoolDiff,
        UdevEngineEvent, UnlockMethod,
    },
};

#[macro_use]
mod macros;

#[allow(clippy::module_inception)]
mod engine;
mod shared;
mod sim_engine;
mod strat_engine;
mod structures;
mod types;
