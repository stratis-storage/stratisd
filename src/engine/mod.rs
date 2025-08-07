// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "extras")]
pub use self::strat_engine::{pool_inspection, ProcessedPathInfos, StratPool};

pub use self::{
    engine::{BlockDev, Engine, Filesystem, KeyActions, Pool, Report},
    shared::{total_allocated, total_used},
    sim_engine::SimEngine,
    strat_engine::{
        create_process_keyring, get_dm, get_dm_init, integrity_meta_space, register_clevis_token,
        set_up_crypt_logging, unshare_mount_namespace, StaticHeader, StaticHeaderResult,
        StratEngine, StratKeyActions, ThinPoolSizeParams, BDA, CLEVIS_TANG_TRUST_URL,
        DEFAULT_CRYPT_DATA_OFFSET_V2,
    },
    structures::{
        AllLockReadGuard, ExclusiveGuard, SharedGuard, SomeLockReadGuard, SomeLockWriteGuard, Table,
    },
    types::{
        ActionAvailability, BlockDevTier, ClevisInfo, CreateAction, DeleteAction, DevUuid, Diff,
        EncryptedDevice, EncryptionInfo, EngineAction, Features, FilesystemUuid, GrowAction,
        InputEncryptionInfo, IntegritySpec, IntegrityTagSpec, KeyDescription, Lockable,
        LockedPoolInfo, LockedPoolsInfo, MappingCreateAction, MappingDeleteAction,
        MaybeInconsistent, Name, OptionalTokenSlotInput, PoolDiff, PoolEncryptionInfo,
        PoolIdentifier, PoolUuid, PropChangeAction, RenameAction, ReportType, SetCreateAction,
        SetDeleteAction, SetUnlockAction, StartAction, StopAction, StoppedPoolInfo,
        StoppedPoolsInfo, StratBlockDevDiff, StratFilesystemDiff, StratPoolDiff,
        StratSigblockVersion, StratisUuid, ThinPoolDiff, ToDisplay, TokenUnlockMethod,
        UdevEngineEvent, UnlockMethod, ValidatedIntegritySpec, DEFAULT_INTEGRITY_JOURNAL_SIZE,
        DEFAULT_INTEGRITY_TAG_SPEC,
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
