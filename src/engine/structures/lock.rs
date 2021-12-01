// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    collections::{HashMap, HashSet, VecDeque},
    fmt::{self, Debug, Display},
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll, Waker},
};

use futures::executor::block_on;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::engine::{
    structures::Table,
    types::{AsUuid, LockKey, Name},
};

pub struct SharedGuard<G>(G);

impl<T, G> Deref for SharedGuard<G>
where
    G: Deref<Target = T>,
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &T {
        &*self.0
    }
}

impl<G> Drop for SharedGuard<G> {
    fn drop(&mut self) {
        trace!("Dropping shared lock {}", type_name::<G>());
    }
}

pub struct ExclusiveGuard<G>(G);

impl<T, G> Deref for ExclusiveGuard<G>
where
    G: Deref<Target = T>,
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &T {
        &*self.0
    }
}

impl<G> DerefMut for ExclusiveGuard<G>
where
    G: DerefMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl<G> Drop for ExclusiveGuard<G> {
    fn drop(&mut self) {
        trace!("Dropping exclusive lock {}", type_name::<G>());
    }
}

#[derive(Debug)]
pub struct Lockable<T>(T);

impl<T> Lockable<Arc<RwLock<T>>> {
    pub fn new_shared(t: T) -> Self {
        Lockable(Arc::new(RwLock::new(t)))
    }
}

impl<T> Lockable<Arc<RwLock<T>>>
where
    T: ?Sized,
{
    pub async fn read(&self) -> SharedGuard<OwnedRwLockReadGuard<T>> {
        trace!("Acquiring shared lock on {}", type_name::<Self>());
        let lock = SharedGuard(Arc::clone(&self.0).read_owned().await);
        trace!("Acquired shared lock on {}", type_name::<Self>());
        lock
    }

    pub fn blocking_read(&self) -> SharedGuard<OwnedRwLockReadGuard<T>> {
        block_on(self.read())
    }

    pub async fn write(&self) -> ExclusiveGuard<OwnedRwLockWriteGuard<T>> {
        trace!("Acquiring exclusive lock on {}", type_name::<Self>());
        let lock = ExclusiveGuard(Arc::clone(&self.0).write_owned().await);
        trace!("Acquired exclusive lock on {}", type_name::<Self>());
        lock
    }

    pub fn blocking_write(&self) -> ExclusiveGuard<OwnedRwLockWriteGuard<T>> {
        block_on(self.write())
    }
}

impl<T> Clone for Lockable<Arc<T>>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        Lockable(Arc::clone(&self.0))
    }
}

/// Data structure containing all state related to the locks acquired and requests to acquire
/// the lock that are waiting to be processed.
#[derive(Debug)]
struct LockRecord<U, T> {
    all_read_locked: u64,
    all_write_locked: bool,
    read_locked: HashMap<U, u64>,
    write_locked: HashSet<U>,
    inner: Table<U, T>,
    waiting: VecDeque<Waiter<U>>,
    woken: HashMap<u64, WaitType<U>>,
}

impl<U, T> LockRecord<U, T>
where
    U: AsUuid,
{
    /// * Asserts that tasks performing an actions either are performing an action immediately
    /// after being spawned or are in the list of woken tasks.
    fn is_woken_or_new(&mut self, wait_type: &WaitType<U>, idx: u64) {
        if self.woken.contains_key(&idx) {
            assert_eq!(self.woken.remove(&idx).as_ref(), Some(wait_type));
        }
    }

    /// * Asserts that tasks performing an actions either are performing an action immediately
    /// after being spawned or are in the list of woken tasks.
    /// * Asserts that the current task never conflicts with tasks that have been woken but
    /// not processed yet.
    fn assert(&mut self, wait_type: &WaitType<U>, idx: u64) {
        self.is_woken_or_new(wait_type, idx);
        assert!(!self.conflicts_with_woken(wait_type));
    }

    /// Convert a name or UUID into a pair of a name and UUID.
    fn get_by_lock_key(&self, lock_key: &LockKey<U>) -> Option<(U, Name)> {
        match lock_key {
            LockKey::Name(ref n) => self.inner.get_by_name(&**n).map(|(u, _)| (u, n.clone())),
            LockKey::Uuid(u) => self.inner.get_by_uuid(*u).map(|(n, _)| (*u, n)),
        }
    }

    /// Add a record for a single element indicating a read lock acquisition.
    fn add_read_lock(&mut self, uuid: U, idx: Option<u64>) {
        match self.read_locked.get_mut(&uuid) {
            Some(counter) => {
                *counter += 1;
            }
            None => {
                self.read_locked.insert(uuid, 1);
            }
        }

        if let Some(i) = idx {
            self.assert(&WaitType::SomeRead(uuid), i);
        }

        trace!("Lock record after acquisition: {}", self);
    }

    /// Remove a record for a single element indicating a read lock acquisition.
    /// Precondition: At least one read lock must be acquired on the given element.
    fn remove_read_lock(&mut self, uuid: U) {
        match self.read_locked.remove(&uuid) {
            Some(counter) => {
                if counter > 1 {
                    self.read_locked.insert(uuid, counter - 1);
                }
            }
            None => panic!("Must have acquired lock and incremented lock count"),
        }
        trace!("Lock record after removal: {}", self);
    }

    /// Add a record for a single element indicating a write lock acquisition.
    fn add_write_lock(&mut self, uuid: U, idx: Option<u64>) {
        self.write_locked.insert(uuid);

        if let Some(i) = idx {
            self.assert(&WaitType::SomeWrite(uuid), i);
        }

        trace!("Lock record after acquisition: {}", self);
    }

    /// Remove a record for a single element indicating a write lock acquisition.
    /// Precondition: Exactly one write lock must be acquired on the given element.
    fn remove_write_lock(&mut self, uuid: &U) {
        assert!(self.write_locked.remove(uuid));
        trace!("Lock record after removal: {}", self);
    }

    /// Add a record for all elements indicating a read lock acquisition.
    fn add_read_all_lock(&mut self, idx: u64) {
        self.all_read_locked += 1;

        self.assert(&WaitType::AllRead, idx);

        trace!("Lock record after acquisition: {}", self);
    }

    /// Remove a record for all elements indicating a read lock acquisition.
    /// Precondition: At least one read lock must be acquired on all elements.
    fn remove_read_all_lock(&mut self) {
        self.all_read_locked = self
            .all_read_locked
            .checked_sub(1)
            .expect("Cannot drop below 0");
        trace!("Lock record after removal: {}", self);
    }

    /// Add a record for all elements indicating a write lock acquisition.
    fn add_write_all_lock(&mut self, idx: u64) {
        self.all_write_locked = true;

        self.assert(&WaitType::AllWrite, idx);

        trace!("Lock record after acquisition: {}", self);
    }

    /// Remove a record for all elements indicating a write lock acquisition.
    /// Precondition: Exactly one write lock must be acquired on all elements.
    fn remove_write_all_lock(&mut self) {
        assert!(self.all_write_locked);
        self.all_write_locked = false;
        trace!("Lock record after removal: {}", self);
    }

    /// Add a lock request to the queue of waiting tasks to be woken up once the lock is
    /// released by any of the current acquisitions.
    fn add_waiter(
        &mut self,
        has_waited: &AtomicBool,
        wait_type: WaitType<U>,
        waker: Waker,
        idx: u64,
    ) {
        // Guard against spurious wake ups.
        if self.waiting.iter().any(|w| w.idx == idx) {
            return;
        }

        self.is_woken_or_new(&wait_type, idx);

        if has_waited.load(Ordering::SeqCst) {
            self.waiting.push_front(Waiter {
                ty: wait_type,
                waker,
                idx,
            });
        } else {
            self.waiting.push_back(Waiter {
                ty: wait_type,
                waker,
                idx,
            });
            has_waited.store(true, Ordering::SeqCst);
        }
        trace!("Lock record after sleep: {}", self);
    }

    /// Returns true if the current request should be put in the wait queue.
    /// * Always returns false if the index for the given request is in the record of woken
    /// tasks.
    /// * Otherwise, returns true if any of the following conditions are met:
    ///   * There are already tasks waiting in the queue.
    ///   * The lock already has a conflicting acquisition.
    ///   * The request conflicts with any tasks that have already been woken up.
    fn should_wait(&self, ty: &WaitType<U>, idx: u64) -> bool {
        if self.woken.contains_key(&idx) {
            trace!(
                "Task with index {}, wait type {:?} was woken and can acquire lock",
                idx,
                ty
            );
            false
        } else {
            let should_wait = !self.waiting.is_empty()
                || self.already_acquired(ty)
                || self.conflicts_with_woken(ty);
            if should_wait {
                trace!(
                    "Putting task with index {}, wait type {:?} to sleep",
                    idx,
                    ty
                );
            } else {
                trace!(
                    "Task with index {}, wait type {:?} can acquire lock",
                    idx,
                    ty
                );
            }
            should_wait
        }
    }

    /// Determines whether two requests conflict.
    fn conflicts(already_woken: &WaitType<U>, ty: &WaitType<U>) -> bool {
        match (already_woken, ty) {
            (WaitType::SomeRead(_), WaitType::SomeRead(_) | WaitType::AllRead) => false,
            (WaitType::SomeRead(uuid1), WaitType::SomeWrite(uuid2)) => uuid1 == uuid2,
            (WaitType::SomeRead(_), _) => true,
            (
                WaitType::SomeWrite(uuid1),
                WaitType::SomeRead(uuid2) | WaitType::SomeWrite(uuid2),
            ) => uuid1 == uuid2,
            (WaitType::SomeWrite(_), _) => true,
            (WaitType::AllRead, WaitType::SomeWrite(_) | WaitType::AllWrite) => true,
            (WaitType::AllRead, _) => false,
            (WaitType::AllWrite, _) => true,
        }
    }

    /// Determines whether the given request conflicts with any of the tasks that have already
    /// been woken up.
    fn conflicts_with_woken(&self, ty: &WaitType<U>) -> bool {
        if self.woken.is_empty() {
            false
        } else {
            self.woken.values().any(|woken| Self::conflicts(woken, ty))
        }
    }

    /// Determines whether the lock has already been acquired by a conflicting request.
    fn already_acquired(&self, ty: &WaitType<U>) -> bool {
        match ty {
            WaitType::SomeRead(uuid) => self.write_locked.contains(uuid) || self.all_write_locked,
            WaitType::SomeWrite(uuid) => {
                self.read_locked.contains_key(uuid)
                    || self.write_locked.contains(uuid)
                    || self.all_read_locked > 0
                    || self.all_write_locked
            }
            WaitType::AllRead => !self.write_locked.is_empty() || self.all_write_locked,
            WaitType::AllWrite => {
                !self.read_locked.is_empty()
                    || !self.write_locked.is_empty()
                    || self.all_read_locked > 0
                    || self.all_write_locked
            }
        }
    }

    /// Determines whether a task should be woken up from the queue.
    /// Returns true if:
    /// * The waiting task does not conflict with any already woken tasks.
    /// * The waiting task does not conflict with any locks currently held.
    fn should_wake(&self) -> bool {
        if let Some(w) = self.waiting.get(0) {
            !self.conflicts_with_woken(&w.ty) && !self.already_acquired(&w.ty)
        } else {
            false
        }
    }

    /// Wake all non-conflicting tasks in the queue and stop at the first conflicting task.
    /// Adds all woken tasks to the record of woken tasks.
    fn wake(&mut self) {
        while self.should_wake() {
            if let Some(w) = self.waiting.pop_front() {
                self.woken.insert(w.idx, w.ty);
                w.waker.wake();
            }
        }
    }
}

impl<U, T> Display for LockRecord<U, T>
where
    U: AsUuid,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockRecord")
            .field("all_read_locked", &self.all_read_locked)
            .field("all_write_locked", &self.all_write_locked)
            .field("read_locked", &self.read_locked)
            .field("write_locked", &self.write_locked)
            .field("waiting", &self.waiting)
            .field("woken", &self.woken)
            .finish()
    }
}

/// A record of the type of a waiting request.
#[derive(Debug, PartialEq)]
enum WaitType<U> {
    SomeRead(U),
    SomeWrite(U),
    AllRead,
    AllWrite,
}

/// A record of a waiting request.
struct Waiter<U> {
    ty: WaitType<U>,
    waker: Waker,
    idx: u64,
}

impl<U> Debug for Waiter<U>
where
    U: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waiter")
            .field("ty", &self.ty)
            .field("idx", &self.idx)
            .finish()
    }
}

/// This data structure is a slightly modified read-write lock. It can either lock all
/// entries contained in the table with read or write permissions, or it can lock
/// individual entries with read or write permissions.
///
/// read() will cause write() on the same element or write_all() to wait.
/// read_all() will cause write() on any element or write_all() to wait.
/// write() will cause write() or read() on the same element or write_all() to wait.
/// write_all() will cause read(), write(), read_all() or write_all() to wait.
#[derive(Debug)]
pub struct AllOrSomeLock<U, T> {
    /// Inner record of acquired locks.
    lock_record: Arc<Mutex<LockRecord<U, T>>>,
    next_idx: Arc<AtomicU64>,
}

impl<U, T> AllOrSomeLock<U, T>
where
    U: AsUuid,
{
    /// Create a new lock for the provided table.
    pub fn new(inner: Table<U, T>) -> Self {
        AllOrSomeLock {
            lock_record: Arc::new(Mutex::new(LockRecord {
                all_read_locked: 0,
                all_write_locked: false,
                read_locked: HashMap::new(),
                write_locked: HashSet::new(),
                inner,
                waiting: VecDeque::new(),
                woken: HashMap::new(),
            })),
            next_idx: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Acquire the mutex protecting the internal lock state.
    fn acquire_mutex(&self) -> MutexGuard<'_, LockRecord<U, T>> {
        self.lock_record
            .lock()
            .expect("lock record mutex only locked internally")
    }
}

impl<U, T> Clone for AllOrSomeLock<U, T> {
    fn clone(&self) -> Self {
        AllOrSomeLock {
            lock_record: Arc::clone(&self.lock_record),
            next_idx: Arc::clone(&self.next_idx),
        }
    }
}

impl<U, T> AllOrSomeLock<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
    /// Issue a read on a single element identified by a name or UUID.
    pub async fn read(&self, key: LockKey<U>) -> Option<SomeLockReadGuard<U, T>> {
        trace!("Acquiring read lock on pool {:?}", key);
        let guard = SomeRead(self.clone(), key, AtomicBool::new(false), self.next_idx()).await;
        if guard.is_some() {
            trace!("Read lock acquired");
        } else {
            trace!("Pool not found");
        }
        guard
    }

    /// Issue a read on all elements.
    pub async fn read_all(&self) -> AllLockReadGuard<U, T> {
        trace!("Acquiring read lock on all pools");
        let guard = AllRead(self.clone(), AtomicBool::new(false), self.next_idx()).await;
        trace!("All read lock acquired");
        guard
    }

    /// Issue a write on a single element identified by a name or UUID.
    pub async fn write(&self, key: LockKey<U>) -> Option<SomeLockWriteGuard<U, T>> {
        trace!("Acquiring write lock on pool {:?}", key);
        let guard = SomeWrite(self.clone(), key, AtomicBool::new(false), self.next_idx()).await;
        if guard.is_some() {
            trace!("Write lock acquired");
        } else {
            trace!("Pool not found");
        }
        guard
    }

    /// Issue a write on all elements.
    pub async fn write_all(&self) -> AllLockWriteGuard<U, T> {
        trace!("Acquiring write lock on all pools");
        let guard = AllWrite(self.clone(), AtomicBool::new(false), self.next_idx()).await;
        trace!("All write lock acquired");
        guard
    }

    /// Returns the index for a future and increments the index count for the next future when
    /// it is created.
    ///
    /// This counter performs wrapping addition so the maximum number of futures supported by
    /// this lock is u64::MAX
    fn next_idx(&self) -> u64 {
        self.next_idx
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |idx| {
                Some((idx + 1) % u64::MAX)
            })
            .expect("Wrapping index update cannot fail")
    }
}

impl<U, T> Default for AllOrSomeLock<U, T>
where
    U: AsUuid,
{
    fn default() -> Self {
        AllOrSomeLock::new(Table::default())
    }
}

/// Future returned by AllOrSomeLock::read().
struct SomeRead<U, T>(AllOrSomeLock<U, T>, LockKey<U>, AtomicBool, u64);

impl<U, T> Unpin for SomeRead<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
}

impl<U, T> Future for SomeRead<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
    type Output = Option<SomeLockReadGuard<U, T>>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock_record = self.0.acquire_mutex();

        let (uuid, name) = if let Some((uuid, name)) = lock_record.get_by_lock_key(&self.1) {
            (uuid, name)
        } else {
            return Poll::Ready(None);
        };

        let wait_type = WaitType::SomeRead(uuid);
        let poll = if lock_record.should_wait(&wait_type, self.3) {
            lock_record.add_waiter(&self.2, wait_type, cxt.waker().clone(), self.3);
            Poll::Pending
        } else {
            lock_record.add_read_lock(uuid, Some(self.3));
            let (_, rf) = lock_record.inner.get_by_uuid(uuid).expect("Checked above");
            Poll::Ready(Some(SomeLockReadGuard(
                self.0.clone(),
                uuid,
                name,
                rf as *const _,
            )))
        };

        poll
    }
}

/// Guard returned by SomeRead future.
pub struct SomeLockReadGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, U, Name, *const T);

impl<U, T> SomeLockReadGuard<U, T>
where
    U: AsUuid,
{
    pub fn as_tuple(&self) -> (Name, U, &T) {
        (
            self.2.clone(),
            self.1,
            unsafe { self.3.as_ref() }.expect("Cannot create null pointer from Rust references"),
        )
    }
}

unsafe impl<U, T> Send for SomeLockReadGuard<U, T>
where
    U: AsUuid + Send,
    T: Send,
{
}

unsafe impl<U, T> Sync for SomeLockReadGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

impl<U, T> Deref for SomeLockReadGuard<U, T>
where
    U: AsUuid,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.3.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for SomeLockReadGuard<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        trace!("Dropping read lock on pool with UUID {}", self.1);
        let mut lock_record = self.0.acquire_mutex();
        lock_record.remove_read_lock(self.1);
        lock_record.wake();
        trace!("Read lock on pool with UUID {} dropped", self.1);
    }
}

/// Future returned by AllOrSomeLock::write().
struct SomeWrite<U, T>(AllOrSomeLock<U, T>, LockKey<U>, AtomicBool, u64);

impl<U, T> Unpin for SomeWrite<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
}

impl<U, T> Future for SomeWrite<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
    type Output = Option<SomeLockWriteGuard<U, T>>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock_record = self.0.acquire_mutex();

        let (uuid, name) = if let Some((uuid, name)) = lock_record.get_by_lock_key(&self.1) {
            (uuid, name)
        } else {
            return Poll::Ready(None);
        };

        let wait_type = WaitType::SomeWrite(uuid);
        let poll = if lock_record.should_wait(&wait_type, self.3) {
            lock_record.add_waiter(&self.2, wait_type, cxt.waker().clone(), self.3);
            Poll::Pending
        } else {
            lock_record.add_write_lock(uuid, Some(self.3));
            let (_, rf) = lock_record.inner.get_by_uuid(uuid).expect("Checked above");
            Poll::Ready(Some(SomeLockWriteGuard(
                self.0.clone(),
                uuid,
                name,
                rf as *const _ as *mut _,
            )))
        };

        poll
    }
}

/// Guard returned by SomeWrite future.
pub struct SomeLockWriteGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, U, Name, *mut T);

impl<U, T> SomeLockWriteGuard<U, T>
where
    U: AsUuid,
{
    pub fn as_tuple(&self) -> (Name, U, &mut T) {
        (
            self.2.clone(),
            self.1,
            unsafe { self.3.as_mut() }.expect("Cannot create null pointer from Rust references"),
        )
    }
}

unsafe impl<U, T> Send for SomeLockWriteGuard<U, T>
where
    U: AsUuid + Send,
    T: Send,
{
}

unsafe impl<U, T> Sync for SomeLockWriteGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

impl<U, T> Deref for SomeLockWriteGuard<U, T>
where
    U: AsUuid,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.3.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> DerefMut for SomeLockWriteGuard<U, T>
where
    U: AsUuid,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.3.as_mut() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for SomeLockWriteGuard<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        trace!("Dropping write lock on pool with UUID {}", self.1);
        let mut lock_record = self.0.acquire_mutex();
        lock_record.remove_write_lock(&self.1);
        lock_record.wake();
        trace!("Write lock on pool with UUID {} dropped", self.1);
    }
}

/// Future returned by AllOrSomeLock::real_all().
struct AllRead<U, T>(AllOrSomeLock<U, T>, AtomicBool, u64);

impl<U, T> Future for AllRead<U, T>
where
    U: AsUuid,
{
    type Output = AllLockReadGuard<U, T>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock_record = self.0.acquire_mutex();

        let wait_type = WaitType::AllRead;
        let poll = if lock_record.should_wait(&wait_type, self.2) {
            lock_record.add_waiter(&self.1, wait_type, cxt.waker().clone(), self.2);
            Poll::Pending
        } else {
            lock_record.add_read_all_lock(self.2);
            Poll::Ready(AllLockReadGuard(
                self.0.clone(),
                &lock_record.inner as *const _,
            ))
        };

        poll
    }
}

/// Guard returned by AllRead future.
pub struct AllLockReadGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, *const Table<U, T>);

impl<U, T> Into<Vec<SomeLockReadGuard<U, T>>> for AllLockReadGuard<U, T>
where
    U: AsUuid,
{
    // Needed because Rust mutability rules will prevent using lock_record mutably in two
    // different closures in the same iterator.
    #[allow(clippy::needless_collect)]
    fn into(self) -> Vec<SomeLockReadGuard<U, T>> {
        let mut lock_record = self.0.acquire_mutex();
        assert!(lock_record.write_locked.is_empty());
        assert!(!lock_record.all_write_locked);

        let guards = lock_record
            .inner
            .iter()
            .map(|(n, u, t)| {
                (
                    *u,
                    SomeLockReadGuard(self.0.clone(), *u, n.clone(), t as *const _),
                )
            })
            .collect::<Vec<_>>();
        guards
            .into_iter()
            .map(|(u, guard)| {
                lock_record.add_read_lock(u, None);
                guard
            })
            .collect::<Vec<_>>()
    }
}

unsafe impl<U, T> Send for AllLockReadGuard<U, T>
where
    U: AsUuid + Send,
    T: Send,
{
}

unsafe impl<U, T> Sync for AllLockReadGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

impl<U, T> Deref for AllLockReadGuard<U, T>
where
    U: AsUuid,
{
    type Target = Table<U, T>;

    fn deref(&self) -> &Self::Target {
        unsafe { self.1.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for AllLockReadGuard<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        trace!("Dropping all read lock");
        let mut lock_record = self.0.acquire_mutex();
        lock_record.remove_read_all_lock();
        lock_record.wake();
        trace!("All read lock dropped");
    }
}

/// Future returned by AllOrSomeLock::write_all().
struct AllWrite<U, T>(AllOrSomeLock<U, T>, AtomicBool, u64);

impl<U, T> Future for AllWrite<U, T>
where
    U: AsUuid,
{
    type Output = AllLockWriteGuard<U, T>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let mut lock_record = self.0.acquire_mutex();

        let wait_type = WaitType::AllWrite;
        let poll = if lock_record.should_wait(&wait_type, self.2) {
            lock_record.add_waiter(&self.1, wait_type, cxt.waker().clone(), self.2);
            Poll::Pending
        } else {
            lock_record.add_write_all_lock(self.2);
            Poll::Ready(AllLockWriteGuard(
                self.0.clone(),
                &lock_record.inner as *const _ as *mut _,
            ))
        };

        poll
    }
}

/// Guard returned by AllWrite future.
pub struct AllLockWriteGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, *mut Table<U, T>);

impl<U, T> Into<Vec<SomeLockWriteGuard<U, T>>> for AllLockWriteGuard<U, T>
where
    U: AsUuid,
{
    // Needed because Rust mutability rules will prevent using lock_record mutably in two
    // different closures in the same iterator.
    #[allow(clippy::needless_collect)]
    fn into(self) -> Vec<SomeLockWriteGuard<U, T>> {
        let mut lock_record = self.0.acquire_mutex();
        assert!(lock_record.read_locked.is_empty());
        assert!(lock_record.write_locked.is_empty());
        assert_eq!(lock_record.all_read_locked, 0);

        let guards = lock_record
            .inner
            .iter()
            .map(|(n, u, t)| {
                (
                    *u,
                    SomeLockWriteGuard(self.0.clone(), *u, n.clone(), t as *const _ as *mut _),
                )
            })
            .collect::<Vec<_>>();
        guards
            .into_iter()
            .map(|(u, guard)| {
                lock_record.add_write_lock(u, None);
                guard
            })
            .collect::<Vec<_>>()
    }
}

unsafe impl<U, T> Send for AllLockWriteGuard<U, T>
where
    U: AsUuid + Send,
    T: Send,
{
}

unsafe impl<U, T> Sync for AllLockWriteGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

impl<U, T> Deref for AllLockWriteGuard<U, T>
where
    U: AsUuid,
{
    type Target = Table<U, T>;

    fn deref(&self) -> &Self::Target {
        unsafe { self.1.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> DerefMut for AllLockWriteGuard<U, T>
where
    U: AsUuid,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.1.as_mut() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for AllLockWriteGuard<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        trace!("Dropping all write lock");
        let mut lock_record = self.0.acquire_mutex();
        lock_record.remove_write_all_lock();
        lock_record.wake();
        trace!("All write lock dropped");
    }
}
