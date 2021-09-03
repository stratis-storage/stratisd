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
    sync::{Arc, Mutex as SyncMutex},
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

macro_rules! lock_mutex {
    ($mutex:expr) => {{
        trace!("Locking internal mutex...");
        let guard = $mutex.lock().expect("mutex only locked internally");
        trace!("Locked internal mutex");
        guard
    }};
}

#[derive(Debug)]
struct LockRecord<U, T> {
    all_read_locked: u64,
    all_write_locked: bool,
    read_locked: HashMap<U, u64>,
    write_locked: HashSet<U>,
    waiting: VecDeque<Waiter<U>>,
    inner: Table<U, T>,
}

impl<U, T> Display for LockRecord<U, T>
where
    U: AsUuid,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LockRecord")
            .field("all_read_locked", &self.all_read_locked)
            .field("all_write_locked", &self.all_write_locked)
            .field("read_locked", &self.read_locked)
            .field("write_locked", &self.write_locked)
            .field("waiting", &self.waiting)
            .finish()
    }
}

#[derive(Debug)]
pub enum WaitType<U> {
    SomeRead(U),
    SomeWrite(U),
    AllRead,
    AllWrite,
}

pub struct Waiter<U> {
    ty: WaitType<U>,
    waker: Waker,
}

impl<U> Debug for Waiter<U>
where
    U: Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.ty)
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
    lock_record: Arc<SyncMutex<LockRecord<U, T>>>,
}

impl<U, T> Clone for AllOrSomeLock<U, T> {
    fn clone(&self) -> Self {
        AllOrSomeLock {
            lock_record: Arc::clone(&self.lock_record),
        }
    }
}

impl<U, T> AllOrSomeLock<U, T>
where
    U: AsUuid,
{
    pub fn new(inner: Table<U, T>) -> Self {
        AllOrSomeLock {
            lock_record: Arc::new(SyncMutex::new(LockRecord {
                all_read_locked: 0,
                all_write_locked: false,
                read_locked: HashMap::new(),
                write_locked: HashSet::new(),
                waiting: VecDeque::new(),
                inner,
            })),
        }
    }
}

impl<U, T> AllOrSomeLock<U, T>
where
    U: AsUuid + Unpin,
    T: Unpin,
{
    pub async fn read(&self, key: LockKey<U>, info: String) -> Option<SomeLockReadGuard<U, T>> {
        trace!(".read() called from {}", info);
        trace!("Acquiring read lock on pool {:?}", key);
        let guard = SomeRead(self.clone(), key, false).await;
        if guard.is_some() {
            trace!("Read lock acquired");
        } else {
            trace!("Pool not found");
        }
        guard
    }

    pub async fn read_all(&self, info: String) -> AllLockReadGuard<U, T> {
        trace!(".read_all() called from {}", info);
        trace!("Acquiring read lock on all pools");
        let guard = AllRead(self.clone(), false).await;
        trace!("All read lock acquired");
        guard
    }

    pub async fn write(&self, key: LockKey<U>, info: String) -> Option<SomeLockWriteGuard<U, T>> {
        trace!(".write() called from {}", info);
        trace!("Acquiring write lock on pool {:?}", key);
        let guard = SomeWrite(self.clone(), key, false).await;
        if guard.is_some() {
            trace!("Read lock acquired");
        } else {
            trace!("Pool not found");
        }
        guard
    }

    pub async fn write_all(&self, info: String) -> AllLockWriteGuard<U, T> {
        trace!(".write_all() called from {}", info);
        trace!("Acquiring write lock on all pools");
        let guard = AllWrite(self.clone(), false).await;
        trace!("All write lock acquired");
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
struct SomeRead<U, T>(AllOrSomeLock<U, T>, LockKey<U>, bool);

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

    fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        trace!("Polling read lock on pool {:?}", self.1);
        let arc = self.0.clone();
        let mut lock_record = lock_mutex!(arc.lock_record);
        if let Some((uuid, name)) = match self.1 {
            LockKey::Name(ref n) => lock_record
                .inner
                .get_by_name(&**n)
                .map(|(u, _)| (u, n.clone())),
            LockKey::Uuid(u) => lock_record.inner.get_by_uuid(u).map(|(n, _)| (u, n)),
        } {
            if (lock_record.all_write_locked || lock_record.write_locked.contains(&uuid))
                || ((lock_record.all_read_locked > 0 || !lock_record.read_locked.is_empty())
                    && !lock_record.waiting.is_empty())
            {
                let waker = cxt.waker().clone();
                if self.2 {
                    lock_record.waiting.push_front(Waiter {
                        ty: WaitType::SomeRead(uuid),
                        waker,
                    });
                } else {
                    lock_record.waiting.push_back(Waiter {
                        ty: WaitType::SomeRead(uuid),
                        waker,
                    });
                    self.2 = true;
                }
                Poll::Pending
            } else {
                match lock_record.read_locked.get_mut(&uuid) {
                    Some(counter) => {
                        *counter += 1;
                    }
                    None => {
                        lock_record.read_locked.insert(uuid, 1);
                    }
                }
                let (_, rf) = lock_record.inner.get_by_uuid(uuid).expect("Checked above");
                Poll::Ready(Some(SomeLockReadGuard(
                    self.0.clone(),
                    uuid,
                    name,
                    rf as *const _,
                )))
            }
        } else {
            Poll::Ready(None)
        }
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
        let mut lock_record = lock_mutex!(self.0.lock_record);
        trace!("Lock record before drop: {}", lock_record);
        match lock_record.read_locked.remove(&self.1) {
            Some(counter) => {
                if counter > 1 {
                    lock_record.read_locked.insert(self.1, counter - 1);
                }
            }
            None => panic!("Must have acquired lock and incremented lock count"),
        }
        if let Some(w) = lock_record.waiting.pop_front() {
            w.waker.wake();
        }
        trace!("Read lock on pool with UUID {} dropped", self.1);
        trace!("Lock record after drop: {}", lock_record);
    }
}

/// Future returned by AllOrSomeLock::write().
struct SomeWrite<U, T>(AllOrSomeLock<U, T>, LockKey<U>, bool);

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

    fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        trace!("Polling write lock on pool {:?}", self.1);
        let arc = self.0.clone();
        let mut lock_record = lock_mutex!(arc.lock_record);
        if let Some((uuid, name)) = match self.1 {
            LockKey::Name(ref n) => lock_record
                .inner
                .get_by_name(&**n)
                .map(|(u, _)| (u, n.clone())),
            LockKey::Uuid(u) => lock_record.inner.get_by_uuid(u).map(|(n, _)| (u, n)),
        } {
            if lock_record.all_write_locked
                || lock_record.write_locked.contains(&uuid)
                || lock_record.all_read_locked > 0
                || lock_record.read_locked.contains_key(&uuid)
            {
                let waker = cxt.waker().clone();
                if self.2 {
                    lock_record.waiting.push_front(Waiter {
                        ty: WaitType::SomeWrite(uuid),
                        waker,
                    });
                } else {
                    lock_record.waiting.push_back(Waiter {
                        ty: WaitType::SomeWrite(uuid),
                        waker,
                    });
                    self.2 = true;
                }
                Poll::Pending
            } else {
                lock_record.write_locked.insert(uuid);
                let (_, rf) = lock_record.inner.get_by_uuid(uuid).expect("Checked above");
                Poll::Ready(Some(SomeLockWriteGuard(
                    self.0.clone(),
                    uuid,
                    name,
                    rf as *const _ as *mut _,
                )))
            }
        } else {
            Poll::Ready(None)
        }
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
        let mut lock_record = lock_mutex!(self.0.lock_record);
        trace!("Lock record before drop: {}", lock_record);
        assert!(lock_record.write_locked.remove(&self.1));
        if let Some(w) = lock_record.waiting.pop_front() {
            w.waker.wake();
        }
        trace!("Write lock on pool with UUID {} dropped", self.1);
        trace!("Lock record after drop: {}", lock_record);
    }
}

/// Future returned by AllOrSomeLock::real_all().
struct AllRead<U, T>(AllOrSomeLock<U, T>, bool);

impl<U, T> Future for AllRead<U, T>
where
    U: AsUuid,
{
    type Output = AllLockReadGuard<U, T>;

    fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        trace!("Polling all read lock");
        let arc = self.0.clone();
        let mut lock_record = lock_mutex!(arc.lock_record);
        if (lock_record.all_write_locked || !lock_record.write_locked.is_empty())
            || ((lock_record.all_read_locked > 0 || !lock_record.read_locked.is_empty())
                && !lock_record.waiting.is_empty())
        {
            let waker = cxt.waker().clone();
            if self.1 {
                lock_record.waiting.push_front(Waiter {
                    ty: WaitType::AllRead,
                    waker,
                });
            } else {
                lock_record.waiting.push_back(Waiter {
                    ty: WaitType::AllRead,
                    waker,
                });
                self.1 = true;
            }
            Poll::Pending
        } else {
            lock_record.all_read_locked += 1;
            Poll::Ready(AllLockReadGuard(
                self.0.clone(),
                &lock_record.inner as *const _,
            ))
        }
    }
}

/// Guard returned by AllRead future.
pub struct AllLockReadGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, *const Table<U, T>);

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
        let mut lock_record = lock_mutex!(self.0.lock_record);
        trace!("Lock record before drop: {}", lock_record);
        lock_record.all_read_locked = lock_record
            .all_read_locked
            .checked_sub(1)
            .expect("Cannot drop below 0");
        if let Some(w) = lock_record.waiting.pop_front() {
            w.waker.wake();
        }
        trace!("All read lock dropped");
        trace!("Lock record after drop: {}", lock_record);
    }
}

/// Future returned by AllOrSomeLock::write_all().
struct AllWrite<U, T>(AllOrSomeLock<U, T>, bool);

impl<U, T> Future for AllWrite<U, T>
where
    U: AsUuid,
{
    type Output = AllLockWriteGuard<U, T>;

    fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
        trace!("Polling all write lock");
        let arc = self.0.clone();
        let mut lock_record = lock_mutex!(arc.lock_record);
        if lock_record.all_write_locked
            || !lock_record.write_locked.is_empty()
            || lock_record.all_read_locked > 0
            || !lock_record.read_locked.is_empty()
        {
            let waker = cxt.waker().clone();
            if self.1 {
                lock_record.waiting.push_front(Waiter {
                    ty: WaitType::AllWrite,
                    waker,
                });
            } else {
                lock_record.waiting.push_back(Waiter {
                    ty: WaitType::AllWrite,
                    waker,
                });
                self.1 = true;
            }
            Poll::Pending
        } else {
            lock_record.all_write_locked = true;
            Poll::Ready(AllLockWriteGuard(
                self.0.clone(),
                &lock_record.inner as *const _ as *mut _,
            ))
        }
    }
}

/// Guard returned by AllWrite future.
pub struct AllLockWriteGuard<U: AsUuid, T>(AllOrSomeLock<U, T>, *mut Table<U, T>);

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
        let mut lock_record = lock_mutex!(self.0.lock_record);
        trace!("Lock record before drop: {}", lock_record);
        assert!(lock_record.all_write_locked);
        lock_record.all_write_locked = false;
        if let Some(w) = lock_record.waiting.pop_front() {
            w.waker.wake();
        }
        trace!("All write lock dropped");
        trace!("Lock record after drop: {}", lock_record);
    }
}
