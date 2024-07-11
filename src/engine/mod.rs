// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "test_extras")]
pub use self::strat_engine::{ProcessedPathInfos, StratPool};
pub use self::{
    engine::{BlockDev, Engine, Filesystem, KeyActions, Pool, Report},
    shared::{total_allocated, total_used},
    sim_engine::SimEngine,
    strat_engine::{
        crypt_metadata_size, get_dm, get_dm_init, register_clevis_token, set_up_crypt_logging,
        unshare_mount_namespace, StaticHeader, StaticHeaderResult, StratEngine, StratKeyActions,
        ThinPoolSizeParams, BDA, CLEVIS_TANG_TRUST_URL,
    },
    structures::{AllLockReadGuard, ExclusiveGuard, SharedGuard, Table},
    types::{
        ActionAvailability, BlockDevTier, ClevisInfo, CreateAction, DeleteAction, DevUuid, Diff,
        EncryptionInfo, EngineAction, FilesystemUuid, GrowAction, KeyDescription, Lockable,
        LockedPoolInfo, LockedPoolsInfo, MappingCreateAction, MappingDeleteAction,
        MaybeInconsistent, Name, PoolDiff, PoolEncryptionInfo, PoolIdentifier, PoolUuid,
        PropChangeAction, RenameAction, ReportType, SetCreateAction, SetDeleteAction,
        SetUnlockAction, StartAction, StopAction, StoppedPoolInfo, StoppedPoolsInfo,
        StratBlockDevDiff, StratFilesystemDiff, StratPoolDiff, StratSigblockVersion, StratisUuid,
        ThinPoolDiff, ToDisplay, UdevEngineEvent, UnlockMethod,
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
