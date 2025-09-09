// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    fmt::{self, Debug, Display},
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll, Waker},
};

use futures::executor::block_on;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::engine::{
    engine::Pool,
    structures::table::{Iter, IterMut, Table},
    types::{AsUuid, Name, PoolIdentifier},
};

pub struct SharedGuard<G>(G);

impl<T, G> Deref for SharedGuard<G>
where
    G: Deref<Target = T>,
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<G> Drop for SharedGuard<G> {
    fn drop(&mut self) {
        trace!("Dropping shared lock on {}", type_name::<G>());
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
        &self.0
    }
}

impl<G> DerefMut for ExclusiveGuard<G>
where
    G: DerefMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        // See: https://github.com/rust-lang/rust-clippy/issues/9763
        #[allow(clippy::explicit_auto_deref)]
        &mut *self.0
    }
}

impl<G> Drop for ExclusiveGuard<G> {
    fn drop(&mut self) {
        trace!("Dropping exclusive lock on {}", type_name::<G>());
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

type Mutexes<'a, U, T> = (
    MutexGuard<'a, LockRecord<U>>,
    MutexGuard<'a, UnsafeCell<Table<U, T>>>,
);

/// Data structure containing all state related to the locks acquired and requests to acquire
/// the lock that are waiting to be processed.
#[derive(Debug)]
struct LockRecord<U> {
    all_read_locked: u64,
    all_write_locked: bool,
    read_locked: HashMap<U, u64>,
    write_locked: HashSet<U>,
    waiting: VecDeque<Waiter<U>>,
    woken: HashMap<u64, WaitType<U>>,
    next_idx: u64,
}

impl<U> LockRecord<U>
where
    U: AsUuid,
{
    /// * Asserts that tasks performing an actions either are performing an action immediately
    ///   after being spawned or are in the list of woken tasks.
    ///
    /// NOTE: This method has the side effect of clearing a woken waiter if it is the waiter that
    /// is currently acquiring the lock.
    fn woken_or_new(&mut self, wait_type: Option<&WaitType<U>>, idx: u64) {
        if self.woken.contains_key(&idx) {
            let woken = self.woken.remove(&idx);
            if let Some(w) = wait_type {
                assert_eq!(woken.as_ref(), Some(w));
            }
        }
    }

    /// * Asserts that tasks performing an actions either are performing an action immediately
    ///   after being spawned or are in the list of woken tasks.
    /// * Asserts that the current task never conflicts with tasks that have been woken but
    ///   not processed yet.
    ///
    /// NOTE: This method has the side effect of clearing a woken waiter if it is the waiter that
    /// is currently acquiring the lock.
    fn pre_acquire_assertion(&mut self, wait_type: &WaitType<U>, idx: u64) {
        self.woken_or_new(Some(wait_type), idx);
        assert!(!self.conflicts_with_woken(wait_type));
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
            self.pre_acquire_assertion(&WaitType::SomeRead(uuid), i);
        }

        trace!("Lock record after acquisition: {self}");
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
        trace!("Lock record after removal: {self}");
    }

    /// Add a record for a single element indicating a write lock acquisition.
    fn add_write_lock(&mut self, uuid: U, idx: Option<u64>) {
        self.write_locked.insert(uuid);

        if let Some(i) = idx {
            self.pre_acquire_assertion(&WaitType::SomeWrite(uuid), i);
        }

        trace!("Lock record after acquisition: {self}");
    }

    /// Remove a record for a single element indicating a write lock acquisition.
    /// Precondition: Exactly one write lock must be acquired on the given element.
    fn remove_write_lock(&mut self, uuid: &U) {
        assert!(self.write_locked.remove(uuid));
        trace!("Lock record after removal: {self}");
    }

    /// Add a record for all elements indicating a read lock acquisition.
    fn add_read_all_lock(&mut self, idx: u64) {
        self.all_read_locked += 1;

        self.pre_acquire_assertion(&WaitType::AllRead, idx);

        trace!("Lock record after acquisition: {self}");
    }

    /// Remove a record for all elements indicating a read lock acquisition.
    /// Precondition: At least one read lock must be acquired on all elements.
    fn remove_read_all_lock(&mut self) {
        self.all_read_locked = self
            .all_read_locked
            .checked_sub(1)
            .expect("Cannot drop below 0");
        trace!("Lock record after removal: {self}");
    }

    /// Add a record for all elements indicating a write lock acquisition.
    fn add_write_all_lock(&mut self, idx: u64) {
        self.all_write_locked = true;

        self.pre_acquire_assertion(&WaitType::AllWrite, idx);

        trace!("Lock record after acquisition: {self}");
    }

    /// Remove a record for all elements indicating a write lock acquisition.
    /// Precondition: Exactly one write lock must be acquired on all elements.
    fn remove_write_all_lock(&mut self) {
        assert!(self.all_write_locked);
        self.all_write_locked = false;
        trace!("Lock record after removal: {self}");
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

        self.woken_or_new(Some(&wait_type), idx);

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
        trace!("Lock record after sleep: {self}");
    }

    /// Returns true if the current request should be put in the wait queue.
    /// * Always returns false if the index for the given request is in the record of woken
    ///   tasks.
    /// * Otherwise, returns true if either of the following conditions are met:
    ///   * The lock already has a conflicting acquisition.
    ///   * The request conflicts with any tasks that have already been woken up.
    fn should_wait(&self, ty: &WaitType<U>, idx: u64) -> bool {
        if self.woken.contains_key(&idx) {
            trace!("Task with index {idx}, wait type {ty:?} was woken and can acquire lock");
            false
        } else {
            let should_wait = self.already_acquired(ty) || self.conflicts_with_woken(ty);
            if should_wait {
                trace!("Putting task with index {idx}, wait type {ty:?} to sleep");
            } else {
                trace!("Task with index {idx}, wait type {ty:?} can acquire lock");
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

    /// Wake all non-conflicting tasks in the queue.
    /// Adds all woken tasks to the record of woken tasks.
    fn wake(&mut self) {
        let mut waiting = VecDeque::new();
        std::mem::swap(&mut waiting, &mut self.waiting);
        for waiter in waiting.drain(..) {
            if !self.conflicts_with_woken(&waiter.ty) && !self.already_acquired(&waiter.ty) {
                self.woken.insert(waiter.idx, waiter.ty);
                waiter.waker.wake();
            } else {
                self.waiting.push_back(waiter);
            }
        }
    }

    /// Remove the internal state of a given future that has been cancelled.
    fn cancel(&mut self, idx: u64) {
        self.waiting = self
            .waiting
            .drain(..)
            .filter(|waiter| waiter.idx != idx)
            .collect::<VecDeque<_>>();
        self.woken = self
            .woken
            .drain()
            .filter(|(i, _)| i != &idx)
            .collect::<HashMap<_, _>>();
    }
}

impl<U> Display for LockRecord<U>
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
            .field("next_idx", &self.next_idx)
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

/// Convert a name or UUID into a pair of a name and UUID.
fn get_by_lock_key<U, T>(
    inner: &UnsafeCell<Table<U, T>>,
    lock_key: &PoolIdentifier<U>,
) -> Option<(U, Name)>
where
    U: AsUuid,
{
    match lock_key {
        PoolIdentifier::Name(ref n) => unsafe { inner.get().as_ref() }
            .and_then(|i| i.get_by_name(n).map(|(u, _)| (u, n.clone()))),
        PoolIdentifier::Uuid(u) => {
            unsafe { inner.get().as_ref() }.and_then(|i| i.get_by_uuid(*u).map(|(n, _)| (*u, n)))
        }
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
    lock_record: Arc<Mutex<LockRecord<U>>>,
    // UnsafeCell is used here to provide interior mutability and avoid any undefined
    // behavior around immutable references being converted to mutable references.
    inner: Arc<Mutex<UnsafeCell<Table<U, T>>>>,
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
                waiting: VecDeque::new(),
                woken: HashMap::new(),
                next_idx: 0,
            })),
            inner: Arc::new(Mutex::new(UnsafeCell::new(inner))),
        }
    }

    /// Acquire the mutex protecting the internal lock state.
    fn acquire_mutex(&self) -> Mutexes<'_, U, T> {
        (
            self.lock_record
                .lock()
                .expect("lock record mutex only locked internally"),
            self.inner
                .lock()
                .expect("inner mutex only locked internally"),
        )
    }

    /// Returns the index for a future and increments the index count for the next future when
    /// it is created.
    ///
    /// This counter performs wrapping addition so the maximum number of futures supported by
    /// this lock is u64::MAX.
    fn next_idx(&self) -> u64 {
        let (mut lock_record, _unused) = self.acquire_mutex();
        let idx = lock_record.next_idx;
        lock_record.next_idx = lock_record.next_idx.wrapping_add(1);
        idx
    }
}

impl<U, T> Clone for AllOrSomeLock<U, T> {
    fn clone(&self) -> Self {
        AllOrSomeLock {
            lock_record: Arc::clone(&self.lock_record),
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<U, T> AllOrSomeLock<U, T>
where
    U: AsUuid,
{
    /// Issue a read on a single element identified by a name or UUID.
    pub async fn read(&self, key: PoolIdentifier<U>) -> Option<SomeLockReadGuard<U, T>> {
        trace!("Acquiring read lock on pool {key:?}");
        let idx = self.next_idx();
        let guard = SomeRead(self.clone(), key, AtomicBool::new(false), idx).await;
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
        let idx = self.next_idx();
        let guard = AllRead(self.clone(), AtomicBool::new(false), idx).await;
        trace!("All read lock acquired");
        guard
    }

    /// Issue a write on a single element identified by a name or UUID.
    pub async fn write(&self, key: PoolIdentifier<U>) -> Option<SomeLockWriteGuard<U, T>> {
        trace!("Acquiring write lock on pool {key:?}");
        let idx = self.next_idx();
        let guard = SomeWrite(self.clone(), key, AtomicBool::new(false), idx).await;
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
        let idx = self.next_idx();
        let guard = AllWrite(self.clone(), AtomicBool::new(false), idx).await;
        trace!("All write lock acquired");
        guard
    }

    /// Issue a modify on element container.
    pub async fn modify_all(&self) -> AllLockModifyGuard<U, T> {
        trace!("Acquiring modify lock on all pools");
        let idx = self.next_idx();
        let guard = AllModify(self.clone(), AtomicBool::new(false), idx).await;
        trace!("All modify lock acquired");
        guard
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
struct SomeRead<U: AsUuid, T>(AllOrSomeLock<U, T>, PoolIdentifier<U>, AtomicBool, u64);

impl<U, T> Future for SomeRead<U, T>
where
    U: AsUuid,
{
    type Output = Option<SomeLockReadGuard<U, T>>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let (mut lock_record, inner) = self.0.acquire_mutex();

        let (uuid, name) = if let Some((uuid, name)) = get_by_lock_key(&*inner, &self.1) {
            (uuid, name)
        } else {
            lock_record.woken_or_new(None, self.3);
            lock_record.wake();
            return Poll::Ready(None);
        };

        let wait_type = WaitType::SomeRead(uuid);
        let poll = if lock_record.should_wait(&wait_type, self.3) {
            lock_record.add_waiter(&self.2, wait_type, cxt.waker().clone(), self.3);
            Poll::Pending
        } else {
            lock_record.add_read_lock(uuid, Some(self.3));
            let (_, rf) = unsafe { inner.get().as_ref() }
                .expect("cannot be null")
                .get_by_uuid(uuid)
                .expect("Checked above");
            Poll::Ready(Some(SomeLockReadGuard(
                Arc::clone(&self.0.lock_record),
                uuid,
                name,
                rf as *const _,
            )))
        };

        poll
    }
}

impl<U, T> Drop for SomeRead<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        let mut lock_record = self
            .0
            .lock_record
            .lock()
            .expect("Mutex only locked internally");
        lock_record.cancel(self.3);
    }
}

/// Guard returned by SomeRead future.
pub struct SomeLockReadGuard<U: AsUuid, T: ?Sized>(Arc<Mutex<LockRecord<U>>>, U, Name, *const T);

impl<U, T> SomeLockReadGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    pub fn as_tuple(&self) -> (Name, U, &T) {
        (
            self.2.clone(),
            self.1,
            unsafe { self.3.as_ref() }.expect("Cannot create null pointer from Rust references"),
        )
    }
}

impl<U, T> SomeLockReadGuard<U, T>
where
    U: AsUuid,
    T: 'static + Pool,
{
    pub fn into_dyn(self) -> SomeLockReadGuard<U, dyn Pool> {
        let (lock_record, uuid, name, ptr) = (
            Arc::clone(&self.0),
            self.1,
            self.2.clone(),
            self.3 as *const dyn Pool,
        );
        std::mem::forget(self);
        SomeLockReadGuard(lock_record, uuid, name, ptr)
    }
}

unsafe impl<U, T> Send for SomeLockReadGuard<U, T>
where
    U: AsUuid + Send,
    T: ?Sized + Send,
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
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.3.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for SomeLockReadGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    fn drop(&mut self) {
        trace!("Dropping read lock on pool with UUID {}", self.1);
        let mut lock_record = self.0.lock().expect("Mutex only locked internally");
        lock_record.remove_read_lock(self.1);
        lock_record.wake();
        trace!("Read lock on pool with UUID {} dropped", self.1);
    }
}

/// Future returned by AllOrSomeLock::write().
struct SomeWrite<U: AsUuid, T>(AllOrSomeLock<U, T>, PoolIdentifier<U>, AtomicBool, u64);

impl<U, T> Future for SomeWrite<U, T>
where
    U: AsUuid,
{
    type Output = Option<SomeLockWriteGuard<U, T>>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let (mut lock_record, inner) = self.0.acquire_mutex();

        let (uuid, name) = if let Some((uuid, name)) = get_by_lock_key(&*inner, &self.1) {
            (uuid, name)
        } else {
            lock_record.woken_or_new(None, self.3);
            lock_record.wake();
            return Poll::Ready(None);
        };

        let wait_type = WaitType::SomeWrite(uuid);
        let poll = if lock_record.should_wait(&wait_type, self.3) {
            lock_record.add_waiter(&self.2, wait_type, cxt.waker().clone(), self.3);
            Poll::Pending
        } else {
            lock_record.add_write_lock(uuid, Some(self.3));
            let (_, rf) = unsafe { inner.get().as_mut() }
                .expect("cannot be null")
                .get_mut_by_uuid(uuid)
                .expect("Checked above");
            Poll::Ready(Some(SomeLockWriteGuard(
                Arc::clone(&self.0.lock_record),
                uuid,
                name,
                rf as *mut _,
                true,
            )))
        };

        poll
    }
}

impl<U, T> Drop for SomeWrite<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        let mut lock_record = self
            .0
            .lock_record
            .lock()
            .expect("Mutex only locked internally");
        lock_record.cancel(self.3);
    }
}

/// Guard returned by SomeWrite future.
pub struct SomeLockWriteGuard<U: AsUuid, T: ?Sized>(
    Arc<Mutex<LockRecord<U>>>,
    U,
    Name,
    *mut T,
    bool,
);

impl<U, T> SomeLockWriteGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    pub fn as_mut_tuple(&mut self) -> (Name, U, &mut T) {
        (
            self.2.clone(),
            self.1,
            unsafe { self.3.as_mut() }.expect("Cannot create null pointer from Rust references"),
        )
    }
}

impl<U, T> SomeLockWriteGuard<U, T>
where
    U: AsUuid,
    T: 'static + Pool,
{
    pub fn into_dyn(mut self) -> SomeLockWriteGuard<U, dyn Pool> {
        self.4 = false;
        SomeLockWriteGuard(
            Arc::clone(&self.0),
            self.1,
            self.2.clone(),
            self.3 as *mut dyn Pool,
            true,
        )
    }
}

unsafe impl<U, T> Send for SomeLockWriteGuard<U, T>
where
    U: AsUuid + Send,
    T: ?Sized + Send,
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
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.3.as_ref() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> DerefMut for SomeLockWriteGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.3.as_mut() }.expect("Cannot create null pointer through references in Rust")
    }
}

impl<U, T> Drop for SomeLockWriteGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    fn drop(&mut self) {
        trace!("Dropping write lock on pool with UUID {}", self.1);
        if self.4 {
            let mut lock_record = self.0.lock().expect("Mutex only locked internally");
            lock_record.remove_write_lock(&self.1);
            lock_record.wake();
        }
        trace!("Write lock on pool with UUID {} dropped", self.1);
    }
}

/// Future returned by AllOrSomeLock::real_all().
struct AllRead<U: AsUuid, T>(AllOrSomeLock<U, T>, AtomicBool, u64);

impl<U, T> Future for AllRead<U, T>
where
    U: AsUuid,
{
    type Output = AllLockReadGuard<U, T>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let (mut lock_record, inner) = self.0.acquire_mutex();

        let wait_type = WaitType::AllRead;
        let poll = if lock_record.should_wait(&wait_type, self.2) {
            lock_record.add_waiter(&self.1, wait_type, cxt.waker().clone(), self.2);
            Poll::Pending
        } else {
            lock_record.add_read_all_lock(self.2);
            Poll::Ready(AllLockReadGuard(
                Arc::clone(&self.0.lock_record),
                unsafe { inner.get().as_ref() }
                    .expect("Not null")
                    .iter()
                    .map(|(n, u, t)| (n.clone(), *u, t as *const _))
                    .collect::<Table<_, _>>(),
                true,
            ))
        };

        poll
    }
}

impl<U, T> Drop for AllRead<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        let mut lock_record = self
            .0
            .lock_record
            .lock()
            .expect("Mutex only locked internally");
        lock_record.cancel(self.2);
    }
}

/// Guard returned by AllRead future.
pub struct AllLockReadGuard<U: AsUuid, T: ?Sized>(
    Arc<Mutex<LockRecord<U>>>,
    Table<U, *const T>,
    bool,
);

impl<U, T> Into<Vec<SomeLockReadGuard<U, T>>> for AllLockReadGuard<U, T>
where
    U: AsUuid,
{
    fn into(self) -> Vec<SomeLockReadGuard<U, T>> {
        let mut lock_record = self.0.lock().expect("Mutex only acquired internally");
        assert!(lock_record.write_locked.is_empty());
        assert!(!lock_record.all_write_locked);

        self.1
            .iter()
            .map(|(n, u, t)| {
                (
                    *u,
                    SomeLockReadGuard(Arc::clone(&self.0), *u, n.clone(), *t as *const _),
                )
            })
            .map(|(u, guard)| {
                lock_record.add_read_lock(u, None);
                guard
            })
            .collect::<Vec<_>>()
    }
}

impl<U, T> AllLockReadGuard<U, T>
where
    U: AsUuid,
    T: 'static + Pool,
{
    pub fn into_dyn(mut self) -> AllLockReadGuard<U, dyn Pool> {
        self.2 = false;
        AllLockReadGuard(
            Arc::clone(&self.0),
            self.1
                .iter()
                .map(|(n, u, t)| (n.clone(), *u, *t as *const dyn Pool))
                .collect::<Table<_, _>>(),
            true,
        )
    }
}

unsafe impl<U, T> Send for AllLockReadGuard<U, T>
where
    U: AsUuid + Send,
    T: ?Sized + Send,
{
}

unsafe impl<U, T> Sync for AllLockReadGuard<U, T>
where
    U: AsUuid + Sync,
    T: ?Sized + Sync,
{
}

pub struct AllLockReadGuardIter<'a, U, T: ?Sized>(Iter<'a, U, *const T>);

impl<'a, U, T> Iterator for AllLockReadGuardIter<'a, U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    type Item = (&'a Name, &'a U, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(n, u, t)| (n, u, unsafe { t.as_ref() }.expect("Not null")))
    }
}

impl<U, T> AllLockReadGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    pub fn get_by_uuid(&self, u: U) -> Option<(Name, &T)> {
        self.1
            .get_by_uuid(u)
            .map(|(n, p)| (n, unsafe { p.as_ref().expect("Not null") }))
    }

    pub fn get_by_name(&self, name: &Name) -> Option<(U, &T)> {
        self.1
            .get_by_name(name)
            .map(|(u, p)| (u, unsafe { p.as_ref().expect("Not null") }))
    }

    pub fn iter(&self) -> AllLockReadGuardIter<'_, U, T> {
        AllLockReadGuardIter(self.1.iter())
    }

    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.1.len()
    }
}

impl<U, T> Drop for AllLockReadGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    fn drop(&mut self) {
        trace!("Dropping all read lock");
        if self.2 {
            let mut lock_record = self.0.lock().expect("Mutex only locked internally");
            lock_record.remove_read_all_lock();
            lock_record.wake();
        }
        trace!("All read lock dropped");
    }
}

/// Future returned by AllOrSomeLock::write_all().
struct AllWrite<U: AsUuid, T>(AllOrSomeLock<U, T>, AtomicBool, u64);

impl<U, T> Future for AllWrite<U, T>
where
    U: AsUuid,
{
    type Output = AllLockWriteGuard<U, T>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let (mut lock_record, inner) = self.0.acquire_mutex();

        let wait_type = WaitType::AllWrite;
        let poll = if lock_record.should_wait(&wait_type, self.2) {
            lock_record.add_waiter(&self.1, wait_type, cxt.waker().clone(), self.2);
            Poll::Pending
        } else {
            lock_record.add_write_all_lock(self.2);
            Poll::Ready(AllLockWriteGuard(
                Arc::clone(&self.0.lock_record),
                unsafe { inner.get().as_mut() }
                    .expect("Not null")
                    .iter_mut()
                    .map(|(n, u, t)| (n.clone(), *u, t as *mut _))
                    .collect::<Table<_, _>>(),
                true,
            ))
        };

        poll
    }
}

impl<U, T> Drop for AllWrite<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        let mut lock_record = self
            .0
            .lock_record
            .lock()
            .expect("Mutex only locked internally");
        lock_record.cancel(self.2);
    }
}

/// Guard returned by AllWrite future.
pub struct AllLockWriteGuard<U: AsUuid, T: ?Sized>(
    Arc<Mutex<LockRecord<U>>>,
    Table<U, *mut T>,
    bool,
);

impl<U, T> Into<Vec<SomeLockWriteGuard<U, T>>> for AllLockWriteGuard<U, T>
where
    U: AsUuid,
{
    fn into(self) -> Vec<SomeLockWriteGuard<U, T>> {
        let mut lock_record = self.0.lock().expect("Mutex only locked internally");
        assert!(lock_record.read_locked.is_empty());
        assert!(lock_record.write_locked.is_empty());
        assert_eq!(lock_record.all_read_locked, 0);

        self.1
            .iter()
            .map(|(n, u, t)| {
                (
                    *u,
                    SomeLockWriteGuard(Arc::clone(&self.0), *u, n.clone(), *t, true),
                )
            })
            .map(|(u, guard)| {
                lock_record.add_write_lock(u, None);
                guard
            })
            .collect::<Vec<_>>()
    }
}

impl<U, T> AllLockWriteGuard<U, T>
where
    U: AsUuid,
    T: 'static + Pool,
{
    pub fn into_dyn(mut self) -> AllLockWriteGuard<U, dyn Pool> {
        self.2 = false;
        AllLockWriteGuard(
            Arc::clone(&self.0),
            self.1
                .iter()
                .map(|(n, u, t)| (n.clone(), *u, *t as *mut dyn Pool))
                .collect::<Table<_, _>>(),
            true,
        )
    }
}

unsafe impl<U, T> Send for AllLockWriteGuard<U, T>
where
    U: AsUuid + Send,
    T: ?Sized + Send,
{
}

unsafe impl<U, T> Sync for AllLockWriteGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

pub struct AllLockWriteGuardIter<'a, U, T: ?Sized>(Iter<'a, U, *mut T>);

impl<'a, U, T> Iterator for AllLockWriteGuardIter<'a, U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    type Item = (&'a Name, &'a U, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(n, u, t)| (n, u, unsafe { t.as_ref() }.expect("Not null")))
    }
}

pub struct AllLockWriteGuardIterMut<'a, U, T: ?Sized>(IterMut<'a, U, *mut T>);

impl<'a, U, T> Iterator for AllLockWriteGuardIterMut<'a, U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    type Item = (&'a Name, &'a U, &'a mut T);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(n, u, t)| (n, u, unsafe { t.as_mut() }.expect("Not null")))
    }
}

impl<U, T> AllLockWriteGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    pub fn get_by_uuid(&self, u: U) -> Option<(Name, &T)> {
        self.1
            .get_by_uuid(u)
            .map(|(n, p)| (n, unsafe { p.as_ref().expect("Not null") }))
    }

    pub fn get_by_name(&self, name: &Name) -> Option<(U, &T)> {
        self.1
            .get_by_name(name)
            .map(|(u, p)| (u, unsafe { p.as_ref().expect("Not null") }))
    }

    pub fn get_mut_by_uuid(&mut self, u: U) -> Option<(Name, &mut T)> {
        self.1
            .get_by_uuid(u)
            .map(|(n, p)| (n, unsafe { p.as_mut().expect("Not null") }))
    }

    pub fn get_mut_by_name(&mut self, name: &Name) -> Option<(U, &mut T)> {
        self.1
            .get_by_name(name)
            .map(|(u, p)| (u, unsafe { p.as_mut().expect("Not null") }))
    }

    pub fn iter(&self) -> AllLockWriteGuardIter<'_, U, T> {
        AllLockWriteGuardIter(self.1.iter())
    }

    pub fn iter_mut(&mut self) -> AllLockWriteGuardIterMut<'_, U, T> {
        AllLockWriteGuardIterMut(self.1.iter_mut())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.1.len()
    }
}

impl<U, T> Drop for AllLockWriteGuard<U, T>
where
    U: AsUuid,
    T: ?Sized,
{
    fn drop(&mut self) {
        trace!("Dropping all write lock");
        if self.2 {
            let mut lock_record = self.0.lock().expect("Mutex only locked internally");
            lock_record.remove_write_all_lock();
            lock_record.wake();
        }
        trace!("All write lock dropped");
    }
}

/// Future returned by AllOrSomeLock::write_all().
struct AllModify<U: AsUuid, T>(AllOrSomeLock<U, T>, AtomicBool, u64);

impl<U, T> Future for AllModify<U, T>
where
    U: AsUuid,
{
    type Output = AllLockModifyGuard<U, T>;

    fn poll(self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        let (mut lock_record, inner) = self.0.acquire_mutex();

        let wait_type = WaitType::AllWrite;
        let poll = if lock_record.should_wait(&wait_type, self.2) {
            lock_record.add_waiter(&self.1, wait_type, cxt.waker().clone(), self.2);
            Poll::Pending
        } else {
            lock_record.add_write_all_lock(self.2);
            Poll::Ready(AllLockModifyGuard(
                Arc::clone(&self.0.lock_record),
                inner.get(),
                true,
            ))
        };

        poll
    }
}

impl<U, T> Drop for AllModify<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        let mut lock_record = self
            .0
            .lock_record
            .lock()
            .expect("Mutex only locked internally");
        lock_record.cancel(self.2);
    }
}

pub struct AllLockModifyGuard<U: AsUuid, T>(Arc<Mutex<LockRecord<U>>>, *mut Table<U, T>, bool);

unsafe impl<U, T> Send for AllLockModifyGuard<U, T>
where
    U: AsUuid + Send,
    T: Send,
{
}

unsafe impl<U, T> Sync for AllLockModifyGuard<U, T>
where
    U: AsUuid + Sync,
    T: Sync,
{
}

impl<U, T> Deref for AllLockModifyGuard<U, T>
where
    U: AsUuid,
{
    type Target = Table<U, T>;

    fn deref(&self) -> &Self::Target {
        unsafe { self.1.as_ref() }.expect("Not null")
    }
}

impl<U, T> DerefMut for AllLockModifyGuard<U, T>
where
    U: AsUuid,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.1.as_mut() }.expect("Not null")
    }
}

impl<U, T> Drop for AllLockModifyGuard<U, T>
where
    U: AsUuid,
{
    fn drop(&mut self) {
        trace!("Dropping all write lock");
        if self.2 {
            let mut lock_record = self.0.lock().expect("Mutex only locked internally");
            lock_record.remove_write_all_lock();
            lock_record.wake();
        }
        trace!("All write lock dropped");
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use futures::poll;

    use crate::engine::types::PoolUuid;

    #[test]
    fn test_cancelled_future() {
        let lock = AllOrSomeLock::new(Table::<PoolUuid, bool>::default());
        let _write_all = test_async!(lock.write_all());
        let read_all = Box::pin(lock.read_all());
        assert!(matches!(
            test_async!(async { poll!(read_all) }),
            Poll::Pending
        ));
        let read_all = Box::pin(lock.read_all());
        assert!(matches!(
            test_async!(async { poll!(read_all) }),
            Poll::Pending
        ));
        let len = lock.lock_record.lock().unwrap().waiting.len();
        assert_eq!(len, 0);
    }
}
