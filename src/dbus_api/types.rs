// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    collections::HashMap,
    fmt::{self, Debug},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use dbus::{
    arg::{RefArg, Variant},
    blocking::SyncConnection,
    Path,
};
use dbus_tree::{DataType, MTSync, ObjectPath, Tree};
use either::Either;
use tokio::sync::{
    mpsc::UnboundedSender as TokioSender, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock,
};

use devicemapper::{Bytes, Sectors};

use crate::{
    dbus_api::{connection::DbusConnectionHandler, tree::DbusTreeHandler, udev::DbusUdevHandler},
    engine::{
        total_allocated, total_used, ActionAvailability, DevUuid, Diff, EncryptionInfo, Engine,
        ExclusiveGuard, FilesystemUuid, Lockable, LockedPoolsInfo, PoolDiff, PoolEncryptionInfo,
        PoolUuid, SharedGuard, StoppedPoolsInfo, StratBlockDevDiff, StratFilesystemDiff,
        StratPoolDiff, StratisUuid, ThinPoolDiff,
    },
};

/// Type for encryption input for multiple token slots.
pub type EncryptionInfos<'a> = (
    Vec<((bool, u32), &'a str)>,
    Vec<((bool, u32), &'a str, &'a str)>,
);

/// Type for lockable D-Bus tree object.
pub type LockableTree = Lockable<Arc<RwLock<Tree<MTSync<TData>, TData>>>>;

/// Type for return value of `GetManagedObjects`.
pub type GetManagedObjects =
    HashMap<dbus::Path<'static>, HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>>;

/// Type representing an acquired read lock for the D-Bus tree.
pub type TreeReadLock = SharedGuard<OwnedRwLockReadGuard<Tree<MTSync<TData>, TData>>>;
/// Type representing an acquired write lock for the D-Bus tree.
pub type TreeWriteLock = ExclusiveGuard<OwnedRwLockWriteGuard<Tree<MTSync<TData>, TData>>>;

/// Type representing all of the handlers for driving the multithreaded D-Bus layer.
pub type DbusHandlers =
    Result<(DbusConnectionHandler, DbusUdevHandler, DbusTreeHandler), dbus::Error>;

/// Type for interfaces parameter for `ObjectManagerInterfacesAdded`. This type cannot be sent
/// over the D-Bus but it is safe to send across threads.
pub type InterfacesAddedThreadSafe =
    HashMap<String, HashMap<String, Variant<Box<dyn RefArg + Send + Sync>>>>;
/// Type for interfaces parameter for `ObjectManagerInterfacesAdded` that can be written to the
/// D-Bus.
pub type InterfacesAdded = HashMap<String, HashMap<String, Variant<Box<dyn RefArg>>>>;
/// Type for interfaces parameter for `ObjectManagerInterfacesRemoved`.
pub type InterfacesRemoved = Vec<String>;

#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub enum DbusErrorEnum {
    OK = 0,
    ERROR = 1,
}

pub const OK_STRING: &str = "Ok";

#[derive(Debug)]
pub enum SignalChange<T> {
    Changed(T),
    Unchanged,
}

impl<T> SignalChange<T> {
    pub fn is_changed(&self) -> bool {
        matches!(self, SignalChange::Changed(_))
    }
}

impl<T> From<Diff<T>> for SignalChange<T> {
    fn from(diff: Diff<T>) -> Self {
        match diff {
            Diff::Changed(t) => SignalChange::Changed(t),
            Diff::Unchanged(_) => SignalChange::Unchanged,
        }
    }
}

#[derive(Debug)]
pub enum DbusAction {
    Add(ObjectPath<MTSync<TData>, TData>, InterfacesAddedThreadSafe),
    Remove(Path<'static>, InterfacesRemoved),
    FsNameChange(Path<'static>, String),
    PoolNameChange(Path<'static>, String),
    PoolAvailActions(Path<'static>, ActionAvailability),
    PoolKeyDescChange(
        Path<'static>,
        Option<Either<(bool, EncryptionInfo), PoolEncryptionInfo>>,
    ),
    PoolClevisInfoChange(
        Path<'static>,
        Option<Either<(bool, EncryptionInfo), PoolEncryptionInfo>>,
    ),
    PoolCacheChange(Path<'static>, bool),
    PoolFsLimitChange(Path<'static>, u64),
    PoolOverprovModeChange(Path<'static>, bool),
    LockedPoolsChange(LockedPoolsInfo),
    StoppedPoolsChange(StoppedPoolsInfo),
    BlockdevUserInfoChange(Path<'static>, Option<String>),
    BlockdevTotalPhysicalSizeChange(Path<'static>, Sectors),
    FsOriginChange(Path<'static>, Option<FilesystemUuid>),
    FsSizeLimitChange(Path<'static>, Option<Sectors>),
    FsMergeScheduledChange(Path<'static>, bool),
    PoolEncryptionChange(Path<'static>, bool),
    FsBackgroundChange(
        FilesystemUuid,
        SignalChange<Option<Bytes>>,
        SignalChange<Bytes>,
    ),
    PoolBackgroundChange(
        PoolUuid,
        SignalChange<Option<Bytes>>,
        SignalChange<Bytes>,
        SignalChange<bool>,
    ),
    PoolForegroundChange(
        Path<'static>,
        SignalChange<Option<Bytes>>,
        SignalChange<Bytes>,
        SignalChange<Bytes>,
        SignalChange<bool>,
    ),
    UdevBackgroundChange(DevUuid, SignalChange<Option<Sectors>>),
}

impl DbusAction {
    /// Convert changed properties from a pool to a series of D-Bus actions.
    pub fn from_pool_diffs(diffs: HashMap<PoolUuid, PoolDiff>) -> Vec<Self> {
        diffs
            .into_iter()
            .map(|(uuid, diff)| {
                let PoolDiff {
                    pool:
                        StratPoolDiff {
                            metadata_size,
                            out_of_alloc_space,
                        },
                    thin_pool:
                        ThinPoolDiff {
                            used,
                            allocated_size,
                        },
                } = diff;

                DbusAction::PoolBackgroundChange(
                    uuid,
                    SignalChange::from(total_used(&used, &metadata_size)),
                    SignalChange::from(total_allocated(&allocated_size, &metadata_size)),
                    SignalChange::from(out_of_alloc_space),
                )
            })
            .collect()
    }

    /// Convert changed properties from filesystems to a series of D-Bus actions.
    pub fn from_fs_diffs(diffs: HashMap<FilesystemUuid, StratFilesystemDiff>) -> Vec<Self> {
        diffs
            .into_iter()
            .map(|(uuid, diff)| {
                let StratFilesystemDiff { size, used } = diff;

                DbusAction::FsBackgroundChange(
                    uuid,
                    SignalChange::from(used),
                    SignalChange::from(size),
                )
            })
            .collect()
    }

    /// Convert changed properties from blockdevs to a series of D-Bus actions.
    pub fn from_bd_diffs(diffs: HashMap<DevUuid, StratBlockDevDiff>) -> Vec<Self> {
        diffs
            .into_iter()
            .map(|(uuid, diff)| {
                let StratBlockDevDiff { size } = diff;

                DbusAction::UdevBackgroundChange(uuid, SignalChange::from(size))
            })
            .collect()
    }
}

/// Context for an object path.
/// Contains the object path of the parent as a Path and the UUID of the
/// object itself.
#[derive(Debug)]
pub struct OPContext {
    pub(super) parent: Path<'static>,
    pub(super) uuid: StratisUuid,
}

impl OPContext {
    pub fn new(parent: Path<'static>, uuid: StratisUuid) -> OPContext {
        OPContext { parent, uuid }
    }
}

pub struct DbusContext {
    next_index: Arc<AtomicU64>,
    pub(super) engine: Arc<dyn Engine>,
    pub(super) sender: TokioSender<DbusAction>,
    connection: Arc<SyncConnection>,
}

impl Clone for DbusContext {
    fn clone(&self) -> Self {
        DbusContext {
            next_index: Arc::clone(&self.next_index),
            engine: self.engine.clone(),
            sender: self.sender.clone(),
            connection: Arc::clone(&self.connection),
        }
    }
}

impl Debug for DbusContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbusContext")
            .field("next_index", &self.next_index)
            .field("engine", &type_name::<Arc<dyn Engine>>())
            .field("sender", &self.sender)
            .finish()
    }
}

impl DbusContext {
    pub fn new(
        engine: Arc<dyn Engine>,
        sender: TokioSender<DbusAction>,
        connection: Arc<SyncConnection>,
    ) -> DbusContext {
        DbusContext {
            engine,
            next_index: Arc::new(AtomicU64::new(0)),
            sender,
            connection,
        }
    }

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&self) -> u64 {
        self.next_index.fetch_add(1, Ordering::SeqCst)
    }

    pub fn push_add(
        &self,
        object_path: ObjectPath<MTSync<TData>, TData>,
        interfaces: InterfacesAddedThreadSafe,
    ) {
        let object_path_name = object_path.get_name().clone();
        if let Err(e) = self.sender.send(DbusAction::Add(object_path, interfaces)) {
            warn!(
                "D-Bus add event could not be sent to the processing thread; the D-Bus \
                server will not be aware of the D-Bus object with path {}: {}",
                object_path_name, e,
            )
        }
    }

    pub fn push_remove(&self, item: &Path<'static>, interfaces: InterfacesRemoved) {
        if let Err(e) = self
            .sender
            .send(DbusAction::Remove(item.clone(), interfaces))
        {
            warn!(
                "D-Bus remove event could not be sent to the processing thread; the D-Bus \
                server will still expect the D-Bus object with path {} to be present: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for Name property and invalidated signal for
    /// Devnode property.
    pub fn push_filesystem_name_change(&self, item: &Path<'static>, new_name: &str) {
        if let Err(e) = self
            .sender
            .send(DbusAction::FsNameChange(item.clone(), new_name.to_string()))
        {
            warn!(
                "D-Bus filesystem name change event could not be sent to the processing thread; \
                no signal will be sent out for pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for KeyDesc property.
    pub fn push_pool_key_desc_change(
        &self,
        item: &Path<'static>,
        ei: Option<Either<(bool, EncryptionInfo), PoolEncryptionInfo>>,
    ) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolKeyDescChange(item.clone(), ei))
        {
            warn!(
                "D-Bus pool key description change event could not be sent to the processing thread; no signal will be sent out for pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for ClevisInfo property.
    pub fn push_pool_clevis_info_change(
        &self,
        item: &Path<'static>,
        ei: Option<Either<(bool, EncryptionInfo), PoolEncryptionInfo>>,
    ) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolClevisInfoChange(item.clone(), ei))
        {
            warn!(
                "D-Bus pool Clevis info change event could not be sent to the processing thread; no signal will be sent out for pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for HasCache property.
    pub fn push_pool_cache_change(&self, item: &Path<'static>, has_cache: bool) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolCacheChange(item.clone(), has_cache))
        {
            warn!(
                "D-Bus pool cache status change event could not be sent to the processing thread; no signal will be sent out for pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool Name property and invalidated signal for
    /// all Devnode properties of child filesystems.
    pub fn push_pool_name_change(&self, item: &Path<'static>, new_name: &str) {
        if let Err(e) = self.sender.send(DbusAction::PoolNameChange(
            item.clone(),
            new_name.to_string(),
        )) {
            warn!(
                "D-Bus pool name change event could not be sent to the processing thread; \
                no signal will be sent out for the name change of pool with path {} or any \
                of its child filesystems: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool FsLimit property.
    pub fn push_pool_fs_limit_change(&self, item: &Path<'static>, new_fs_limit: u64) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolFsLimitChange(item.clone(), new_fs_limit))
        {
            warn!(
                "D-Bus pool filesystem limit change event could not be sent to the processing thread; no signal will be sent out for the filesystem limit change of pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool SizeLimit property.
    pub fn push_fs_size_limit_change(&self, item: &Path<'static>, new_size_limit: Option<Sectors>) {
        if let Err(e) = self
            .sender
            .send(DbusAction::FsSizeLimitChange(item.clone(), new_size_limit))
        {
            warn!(
                "D-Bus filesystem size limit change event could not be sent to the processing thread; no signal will be sent out for the size limit change of filesystem with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool overprovisioning mode property.
    pub fn push_pool_overprov_mode_change(&self, item: &Path<'static>, new_mode: bool) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolOverprovModeChange(item.clone(), new_mode))
        {
            warn!(
                "D-Bus pool overprovisioning mode change event could not be sent to the processing thread; no signal will be sent out for the filesystem overprovisioning mode of pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for pool available actions state.
    pub fn push_pool_avail_actions(&self, item: &Path<'static>, avail_actions: ActionAvailability) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolAvailActions(item.clone(), avail_actions))
        {
            warn!(
                "D-Bus pool available actions status change event could not be sent to the processing thread; no signal will be sent out for the pool available actions status change of pool with path {}: {}",
                item, e,
            )
        }
    }

    /// Send changed signal for changed locked pool state.
    pub fn push_locked_pools(&self, locked_pools: LockedPoolsInfo) {
        if let Err(e) = self
            .sender
            .send(DbusAction::LockedPoolsChange(locked_pools))
        {
            warn!(
                "Locked pool change event could not be sent to the processing thread; no signal will be sent out for the locked pool state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed stopped pool state.
    pub fn push_stopped_pools(&self, stopped_pools: StoppedPoolsInfo) {
        if let Err(e) = self
            .sender
            .send(DbusAction::StoppedPoolsChange(stopped_pools))
        {
            warn!(
                "Stopped pool change event could not be sent to the processing thread; no signal will be sent out for the stopped pool state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed blockdev user info property.
    pub fn push_blockdev_user_info_change(&self, path: &Path<'static>, user_info: Option<String>) {
        if let Err(e) = self
            .sender
            .send(DbusAction::BlockdevUserInfoChange(path.clone(), user_info))
        {
            warn!(
                "Block device User info change event could not be sent to the processing thread; no signal will be sent out for the block device user info state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed blockdev total size property.
    pub fn push_blockdev_total_physical_size_change(
        &self,
        path: &Path<'static>,
        total_physical_size: Sectors,
    ) {
        if let Err(e) = self
            .sender
            .send(DbusAction::BlockdevTotalPhysicalSizeChange(
                path.clone(),
                total_physical_size,
            ))
        {
            warn!(
                "Block device total physical size change event could not be sent to the processing thread; no signal will be sent out for the block device total physical size state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed encryption status of pool.
    pub fn push_pool_encryption_status_change(&self, path: &Path<'static>, encrypted: bool) {
        if let Err(e) = self
            .sender
            .send(DbusAction::PoolEncryptionChange(path.clone(), encrypted))
        {
            warn!(
                "Encryption status change event could not be sent to the processing thread; no signal will be sent out for the encryption status state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed pool properties when blockdevs are
    /// added.
    pub fn push_pool_foreground_change(
        &self,
        path: &Path<'static>,
        new_used: Diff<Option<Bytes>>,
        new_alloc: Diff<Bytes>,
        new_size: Diff<Bytes>,
        out_of_alloc_space: Diff<bool>,
    ) {
        if let Err(e) = self.sender.send(DbusAction::PoolForegroundChange(
            path.clone(),
            SignalChange::from(new_used),
            SignalChange::from(new_alloc),
            SignalChange::from(new_size),
            SignalChange::from(out_of_alloc_space),
        )) {
            warn!(
                "Pool foreground change event could not be sent to the processing thread; no signal will be sent out for the pool allocated size state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for changed filesystem origin property.
    pub fn push_filesystem_origin_change(
        &self,
        path: &Path<'static>,
        origin: Option<FilesystemUuid>,
    ) {
        if let Err(e) = self
            .sender
            .send(DbusAction::FsOriginChange(path.clone(), origin))
        {
            warn!(
                "Filesystem origin change event could not be sent to the processing thread; no signal will be sent out for the filesystem origin state change: {}",
                e,
            )
        }
    }

    /// Send changed signal for pool MergeScheduled property.
    pub fn push_fs_merge_scheduled_change(&self, item: &Path<'static>, new_merge_scheduled: bool) {
        if let Err(e) = self.sender.send(DbusAction::FsMergeScheduledChange(
            item.clone(),
            new_merge_scheduled,
        )) {
            warn!(
                "D-Bus filesystem merge scheduled change event could not be sent to the processing thread; no signal will be sent out for the merge scheduled change of filesystem with path {item}: {e}"
            )
        }
    }
}

#[derive(Debug)]
pub struct TData;

impl Default for TData {
    fn default() -> Self {
        TData
    }
}

impl DataType for TData {
    type Tree = DbusContext;
    type ObjectPath = Option<OPContext>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}
