// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    collections::{hash_map, HashMap},
    fmt,
    iter::IntoIterator,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use futures::executor::block_on;
use tokio::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::engine::types::{AsUuid, Name};

/// Map UUID and name to T items.
pub struct Table<U, T> {
    name_to_uuid: HashMap<Name, U>,
    items: HashMap<U, (Name, T)>,
}

impl<U, T> fmt::Debug for Table<U, T>
where
    U: fmt::Debug,
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_map()
            .entries(
                self.items
                    .iter()
                    .map(|(uuid, (name, item))| ((name.to_string(), uuid), item)),
            )
            .finish()
    }
}

impl<U, T> Default for Table<U, T>
where
    U: AsUuid,
{
    fn default() -> Table<U, T> {
        Table {
            name_to_uuid: HashMap::default(),
            items: HashMap::default(),
        }
    }
}

pub struct Iter<'a, U: 'a, T: 'a> {
    items: hash_map::Iter<'a, U, (Name, T)>,
}

impl<'a, U, T> Iterator for Iter<'a, U, T>
where
    U: AsUuid,
{
    type Item = (&'a Name, &'a U, &'a T);

    #[inline]
    fn next(&mut self) -> Option<(&'a Name, &'a U, &'a T)> {
        self.items
            .next()
            .map(|(uuid, &(ref name, ref item))| (&*name, uuid, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

pub struct IterMut<'a, U: 'a, T: 'a> {
    items: hash_map::IterMut<'a, U, (Name, T)>,
}

impl<'a, U, T> Iterator for IterMut<'a, U, T>
where
    U: AsUuid,
{
    type Item = (&'a Name, &'a U, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<(&'a Name, &'a U, &'a mut T)> {
        self.items
            .next()
            .map(|(uuid, &mut (ref name, ref mut item))| (&*name, uuid, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

pub struct IntoIter<U, T> {
    items: hash_map::IntoIter<U, (Name, T)>,
}

impl<U, T> Iterator for IntoIter<U, T>
where
    U: AsUuid,
{
    type Item = (Name, U, T);

    #[inline]
    fn next(&mut self) -> Option<(Name, U, T)> {
        self.items
            .next()
            .map(|(uuid, (name, item))| (name, uuid, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

impl<U, T> IntoIterator for Table<U, T>
where
    U: AsUuid,
{
    type Item = (Name, U, T);
    type IntoIter = IntoIter<U, T>;

    fn into_iter(self) -> IntoIter<U, T> {
        self.into_iter()
    }
}

impl<'a, U, T> IntoIterator for &'a Table<U, T>
where
    U: AsUuid,
{
    type Item = (&'a Name, &'a U, &'a T);
    type IntoIter = Iter<'a, U, T>;

    fn into_iter(self) -> Iter<'a, U, T> {
        self.iter()
    }
}

impl<'a, U, T> IntoIterator for &'a mut Table<U, T>
where
    U: AsUuid,
{
    type Item = (&'a Name, &'a U, &'a mut T);
    type IntoIter = IterMut<'a, U, T>;

    fn into_iter(self) -> IterMut<'a, U, T> {
        self.iter_mut()
    }
}

/// All operations are O(1), although Name lookups are slightly disadvantaged
/// vs. Uuid lookups. In order to rename a T item, it must be removed,
/// renamed, and reinserted under the new name.
impl<U, T> Table<U, T>
where
    U: AsUuid,
{
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> Iter<U, T> {
        Iter {
            items: self.items.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<U, T> {
        IterMut {
            items: self.items.iter_mut(),
        }
    }

    pub fn into_iter(self) -> IntoIter<U, T> {
        IntoIter {
            items: self.items.into_iter(),
        }
    }

    /// Returns true if map has an item corresponding to this name, else false.
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_to_uuid.contains_key(name)
    }

    /// Returns true if map has an item corresponding to this uuid, else false.
    pub fn contains_uuid(&self, uuid: U) -> bool {
        self.items.contains_key(&uuid)
    }

    /// Get item by name.
    pub fn get_by_name(&self, name: &str) -> Option<(U, &T)> {
        self.name_to_uuid
            .get(&*name)
            .and_then(|uuid| self.items.get(uuid).map(|&(_, ref item)| (*uuid, item)))
    }

    /// Get item by uuid.
    pub fn get_by_uuid(&self, uuid: U) -> Option<(Name, &T)> {
        self.items
            .get(&uuid)
            .map(|&(ref name, ref item)| (name.clone(), item))
    }

    /// Get mutable item by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<(U, &mut T)> {
        let uuid = match self.name_to_uuid.get(name) {
            Some(uuid) => uuid,
            None => return None,
        };
        self.items
            .get_mut(uuid)
            .map(|&mut (_, ref mut item)| (*uuid, item))
    }

    /// Get mutable item by uuid.
    pub fn get_mut_by_uuid(&mut self, uuid: U) -> Option<(Name, &mut T)> {
        self.items
            .get_mut(&uuid)
            .map(|&mut (ref name, ref mut item)| (name.clone(), item))
    }

    /// Removes the item corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<(U, T)> {
        self.name_to_uuid
            .remove(name)
            .and_then(|uuid| self.items.remove(&uuid).map(|(_, item)| (uuid, item)))
    }

    /// Removes the item corresponding to the uuid if there is one.
    pub fn remove_by_uuid(&mut self, uuid: U) -> Option<(Name, T)> {
        self.items.remove(&uuid).map(|(name, item)| {
            self.name_to_uuid.remove(&name);
            (name, item)
        })
    }

    /// Inserts an item for given uuid and name.
    /// Possibly returns the items displaced.
    /// If two items are displaced, the one displaced by matching name is
    /// returned first.
    pub fn insert(&mut self, name: Name, uuid: U, item: T) -> Option<Vec<(Name, U, T)>> {
        let old_uuid = self.name_to_uuid.insert(name.clone(), uuid);
        let old_pair = self.items.insert(uuid, (name.clone(), item));
        match (old_uuid, old_pair) {
            // Two possibilities: One entry with same name and uuid ejected OR
            // two entries, one with same name and different uuid and one with
            // same uuid and different name.
            (Some(old_uuid), Some((old_name, old_item))) => Some(if old_uuid == uuid {
                assert_eq!(old_name, name);
                vec![(name, uuid, old_item)]
            } else {
                assert_ne!(old_name, name);
                let other_uuid = self
                    .name_to_uuid
                    .remove(&old_name)
                    .expect("invariant requires existence");
                let (other_name, other_item) = self
                    .items
                    .remove(&old_uuid)
                    .expect("invariant requires existence");
                assert_eq!(other_name, name);
                assert_eq!(other_uuid, uuid);
                vec![(name, old_uuid, other_item), (old_name, uuid, old_item)]
            }),
            // entry with same name but different uuid ejected
            (Some(old_uuid), None) => {
                let (other_name, other_item) = self
                    .items
                    .remove(&old_uuid)
                    .expect("invariant requires existence");
                assert_eq!(other_name, name);
                Some(vec![(name, old_uuid, other_item)])
            }
            // entry with same uuid but different name ejected
            (None, Some((old_name, old_item))) => {
                let other_uuid = self
                    .name_to_uuid
                    .remove(&old_name)
                    .expect("invariant requires existence");
                assert_eq!(other_uuid, uuid);
                Some(vec![(old_name, uuid, old_item)])
            }
            // nothing ejected
            (None, None) => None,
        }
    }
}

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

pub struct Lockable<T>(T);

impl<T> Lockable<Arc<Mutex<T>>> {
    pub fn new_exclusive(t: T) -> Lockable<Arc<Mutex<T>>> {
        Lockable(Arc::new(Mutex::new(t)))
    }
}

impl<T> Lockable<Arc<RwLock<T>>> {
    pub fn new_shared(t: T) -> Self {
        Lockable(Arc::new(RwLock::new(t)))
    }
}

impl<T> Lockable<Arc<Mutex<T>>>
where
    T: ?Sized,
{
    pub async fn lock(&self) -> ExclusiveGuard<MutexGuard<'_, T>> {
        trace!("Acquiring exclusive lock on {}", type_name::<Self>());
        let lock = ExclusiveGuard(self.0.lock().await);
        trace!("Acquired exclusive lock on {}", type_name::<Self>());
        lock
    }

    pub fn blocking_lock(&self) -> ExclusiveGuard<MutexGuard<'_, T>> {
        block_on(self.lock())
    }
}

impl<T> Lockable<Arc<RwLock<T>>>
where
    T: ?Sized,
{
    pub async fn read(&self) -> SharedGuard<RwLockReadGuard<'_, T>> {
        trace!("Acquiring shared lock on {}", type_name::<Self>());
        let lock = SharedGuard(self.0.read().await);
        trace!("Acquired shared lock on {}", type_name::<Self>());
        lock
    }

    pub fn blocking_read(&self) -> SharedGuard<RwLockReadGuard<'_, T>> {
        block_on(self.read())
    }

    pub async fn write(&self) -> ExclusiveGuard<RwLockWriteGuard<'_, T>> {
        trace!("Acquiring exclusive lock on {}", type_name::<Self>());
        let lock = ExclusiveGuard(self.0.write().await);
        trace!("Acquired exclusive lock on {}", type_name::<Self>());
        lock
    }

    pub fn blocking_write(&self) -> ExclusiveGuard<RwLockWriteGuard<'_, T>> {
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

#[allow(dead_code)]
mod table_lock {
    use std::{
        collections::{HashMap, HashSet, VecDeque},
        future::Future,
        ops::{Deref, DerefMut},
        pin::Pin,
        sync::Mutex as SyncMutex,
        task::{Context, Poll, Waker},
    };

    use crate::engine::{
        structures::Table,
        types::{AsUuid, Name},
    };

    #[derive(Debug)]
    struct LockRecord<U, T> {
        all_read_locked: u64,
        all_write_locked: bool,
        read_locked: HashMap<U, u64>,
        write_locked: HashSet<U>,
        waiting: VecDeque<Waker>,
        inner: Table<U, T>,
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
        lock_record: SyncMutex<LockRecord<U, T>>,
    }

    impl<U, T> AllOrSomeLock<U, T>
    where
        U: AsUuid,
    {
        pub fn new(inner: Table<U, T>) -> Self {
            AllOrSomeLock {
                lock_record: SyncMutex::new(LockRecord {
                    all_read_locked: 0,
                    all_write_locked: false,
                    read_locked: HashMap::new(),
                    write_locked: HashSet::new(),
                    waiting: VecDeque::new(),
                    inner,
                }),
            }
        }
    }

    impl<U, T> AllOrSomeLock<U, T>
    where
        U: AsUuid + Unpin,
        T: Unpin,
    {
        pub async fn read(&self, uuid: U) -> Option<SomeLockReadGuard<'_, U, T>> {
            SomeRead(self, uuid, false).await
        }

        pub async fn read_all(&self) -> AllLockReadGuard<'_, U, T> {
            AllRead(self, false).await
        }

        pub async fn write(&self, uuid: U) -> Option<SomeLockWriteGuard<'_, U, T>> {
            SomeWrite(self, uuid, false).await
        }

        pub async fn write_all(&self) -> AllLockWriteGuard<'_, U, T> {
            AllWrite(self, false).await
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
    struct SomeRead<'a, U, T>(&'a AllOrSomeLock<U, T>, U, bool);

    impl<'a, U, T> Unpin for SomeRead<'a, U, T>
    where
        U: AsUuid + Unpin,
        T: Unpin,
    {
    }

    impl<'a, U, T> Future for SomeRead<'a, U, T>
    where
        U: AsUuid + Unpin,
        T: Unpin,
    {
        type Output = Option<SomeLockReadGuard<'a, U, T>>;

        fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            if lock_record.all_write_locked || lock_record.write_locked.contains(&self.1) {
                let waker = cxt.waker().clone();
                if self.2 {
                    lock_record.waiting.push_front(waker);
                } else {
                    lock_record.waiting.push_back(waker);
                    self.2 = true;
                }
                Poll::Pending
            } else {
                match lock_record.read_locked.get_mut(&self.1) {
                    Some(counter) => {
                        *counter += 1;
                    }
                    None => {
                        lock_record.read_locked.insert(self.1, 1);
                    }
                }
                Poll::Ready(
                    lock_record
                        .inner
                        .get_by_uuid(self.1)
                        .map(|(name, rf)| SomeLockReadGuard(self.0, self.1, name, rf as *const _)),
                )
            }
        }
    }

    /// Guard returned by SomeRead future.
    pub struct SomeLockReadGuard<'a, U: AsUuid, T>(&'a AllOrSomeLock<U, T>, U, Name, *const T);

    impl<'a, U, T> SomeLockReadGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn get_name(&self) -> &Name {
            &self.2
        }
    }

    unsafe impl<'a, U, T> Send for SomeLockReadGuard<'a, U, T>
    where
        U: AsUuid + Send,
        T: Send,
    {
    }

    impl<'a, U, T> Deref for SomeLockReadGuard<'a, U, T>
    where
        U: AsUuid,
    {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            unsafe { self.3.as_ref() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> Drop for SomeLockReadGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn drop(&mut self) {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            match lock_record.read_locked.remove(&self.1) {
                Some(counter) => {
                    if counter > 1 {
                        lock_record.read_locked.insert(self.1, counter - 1);
                    }
                }
                None => panic!("Must have acquired lock and incremented lock count"),
            }
        }
    }

    /// Future returned by AllOrSomeLock::write().
    struct SomeWrite<'a, U, T>(&'a AllOrSomeLock<U, T>, U, bool);

    impl<'a, U, T> Unpin for SomeWrite<'a, U, T>
    where
        U: AsUuid + Unpin,
        T: Unpin,
    {
    }

    impl<'a, U, T> Future for SomeWrite<'a, U, T>
    where
        U: AsUuid + Unpin,
        T: Unpin,
    {
        type Output = Option<SomeLockWriteGuard<'a, U, T>>;

        fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            if lock_record.all_write_locked
                || lock_record.write_locked.contains(&self.1)
                || lock_record.all_read_locked > 0
                || lock_record.read_locked.contains_key(&self.1)
            {
                let waker = cxt.waker().clone();
                if self.2 {
                    lock_record.waiting.push_front(waker);
                } else {
                    lock_record.waiting.push_back(waker);
                    self.2 = true;
                }
                Poll::Pending
            } else {
                lock_record.write_locked.insert(self.1);
                Poll::Ready(lock_record.inner.get_by_uuid(self.1).map(|(name, rf)| {
                    SomeLockWriteGuard(self.0, self.1, name, rf as *const _ as *mut _)
                }))
            }
        }
    }

    /// Guard returned by SomeWrite future.
    pub struct SomeLockWriteGuard<'a, U: AsUuid, T>(&'a AllOrSomeLock<U, T>, U, Name, *mut T);

    impl<'a, U, T> SomeLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn get_name(&self) -> &Name {
            &self.2
        }
    }

    unsafe impl<'a, U, T> Send for SomeLockWriteGuard<'a, U, T> where U: AsUuid {}

    impl<'a, U, T> Deref for SomeLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            unsafe { self.3.as_ref() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> DerefMut for SomeLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn deref_mut(&mut self) -> &mut Self::Target {
            unsafe { self.3.as_mut() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> Drop for SomeLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn drop(&mut self) {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            assert!(lock_record.write_locked.remove(&self.1));
            if let Some(w) = lock_record.waiting.pop_front() {
                w.wake();
            }
        }
    }

    /// Future returned by AllOrSomeLock::real_all().
    struct AllRead<'a, U, T>(&'a AllOrSomeLock<U, T>, bool);

    impl<'a, U, T> Future for AllRead<'a, U, T> {
        type Output = AllLockReadGuard<'a, U, T>;

        fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            if lock_record.all_write_locked || !lock_record.write_locked.is_empty() {
                let waker = cxt.waker().clone();
                if self.1 {
                    lock_record.waiting.push_front(waker);
                } else {
                    lock_record.waiting.push_back(waker);
                    self.1 = true;
                }
                Poll::Pending
            } else {
                lock_record.all_read_locked += 1;
                Poll::Ready(AllLockReadGuard(self.0, &lock_record.inner as *const _))
            }
        }
    }

    /// Guard returned by AllRead future.
    pub struct AllLockReadGuard<'a, U, T>(&'a AllOrSomeLock<U, T>, *const Table<U, T>);

    unsafe impl<'a, U, T> Send for AllLockReadGuard<'a, U, T> {}

    impl<'a, U, T> Deref for AllLockReadGuard<'a, U, T>
    where
        U: AsUuid,
    {
        type Target = Table<U, T>;

        fn deref(&self) -> &Self::Target {
            unsafe { self.1.as_ref() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> Drop for AllLockReadGuard<'a, U, T> {
        fn drop(&mut self) {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            assert!(lock_record.all_read_locked.checked_sub(1).is_some());
            if let Some(w) = lock_record.waiting.pop_front() {
                w.wake();
            }
        }
    }

    /// Future returned by AllOrSomeLock::write_all().
    struct AllWrite<'a, U, T>(&'a AllOrSomeLock<U, T>, bool);

    impl<'a, U, T> Future for AllWrite<'a, U, T> {
        type Output = AllLockWriteGuard<'a, U, T>;

        fn poll(mut self: Pin<&mut Self>, cxt: &mut Context<'_>) -> Poll<Self::Output> {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            if lock_record.all_write_locked
                || !lock_record.write_locked.is_empty()
                || lock_record.all_read_locked > 0
                || !lock_record.read_locked.is_empty()
            {
                let waker = cxt.waker().clone();
                if self.1 {
                    lock_record.waiting.push_front(waker);
                } else {
                    lock_record.waiting.push_back(waker);
                    self.1 = true;
                }
                Poll::Pending
            } else {
                lock_record.all_write_locked = true;
                Poll::Ready(AllLockWriteGuard(
                    self.0,
                    &lock_record.inner as *const _ as *mut _,
                ))
            }
        }
    }

    /// Guard returned by AllWrite future.
    pub struct AllLockWriteGuard<'a, U, T>(&'a AllOrSomeLock<U, T>, *mut Table<U, T>);

    unsafe impl<'a, U, T> Send for AllLockWriteGuard<'a, U, T> {}

    impl<'a, U, T> Deref for AllLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        type Target = Table<U, T>;

        fn deref(&self) -> &Self::Target {
            unsafe { self.1.as_ref() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> DerefMut for AllLockWriteGuard<'a, U, T>
    where
        U: AsUuid,
    {
        fn deref_mut(&mut self) -> &mut Self::Target {
            unsafe { self.1.as_mut() }
                .expect("Cannot create null pointer through references in Rust")
        }
    }

    impl<'a, U, T> Drop for AllLockWriteGuard<'a, U, T> {
        fn drop(&mut self) {
            let mut lock_record = self
                .0
                .lock_record
                .lock()
                .expect("mutex only locked internally");
            assert!(lock_record.all_write_locked);
            lock_record.all_write_locked = false;
            if let Some(w) = lock_record.waiting.pop_front() {
                w.wake();
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::engine::{types::PoolUuid, Name};

    use super::*;

    #[derive(Debug)]
    struct TestThing {
        stuff: u32,
    }

    // A global invariant checker for the table.
    // Verifies proper relationship between internal data structures.
    fn table_invariant<U, T>(table: &Table<U, T>)
    where
        U: AsUuid,
    {
        for (uuid, &(ref name, _)) in &table.items {
            assert_eq!(uuid, &table.name_to_uuid[name])
        }

        for (name, uuid) in &table.name_to_uuid {
            assert_eq!(name, &table.items[uuid].0);
        }

        // No extra garbage
        assert_eq!(table.name_to_uuid.len(), table.items.len())
    }

    impl TestThing {
        pub fn new() -> TestThing {
            TestThing {
                stuff: rand::random::<u32>(),
            }
        }
    }

    #[test]
    /// Remove a test object by its uuid.
    /// Mutate the removed test object.
    /// Verify that the table is now empty and that removing by name yields
    /// no result.
    fn remove_existing_item() {
        let mut t: Table<PoolUuid, TestThing> = Table::default();
        table_invariant(&t);

        let uuid = PoolUuid::new_v4();
        let name = "name";
        t.insert(Name::new(name.to_owned()), uuid, TestThing::new());
        table_invariant(&t);

        assert!(t.get_by_name(name).is_some());
        assert!(t.get_by_uuid(uuid).is_some());
        let thing = t.remove_by_uuid(uuid);
        table_invariant(&t);
        assert!(thing.is_some());
        let mut thing = thing.unwrap();
        thing.1.stuff = 0;
        assert!(t.is_empty());
        assert_matches!(t.remove_by_name(name), None);
        table_invariant(&t);

        assert_matches!(t.get_by_name(name), None);
        assert_matches!(t.get_by_uuid(uuid), None);
    }

    #[test]
    /// Insert a thing and then insert another thing with same keys.
    /// The previously inserted thing should be returned.
    /// You can't insert the identical thing, because that would be a move.
    /// This is good, because then you can't have a thing that is both in
    /// the table and not in the table.
    fn insert_same_keys() {
        let mut t: Table<PoolUuid, TestThing> = Table::default();
        table_invariant(&t);

        let uuid = PoolUuid::new_v4();
        let name = "name";
        let thing = TestThing::new();
        let thing_key = thing.stuff;
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);

        // There was nothing previously, so displaced must be empty.
        assert_matches!(displaced, None);

        // t now contains the inserted thing.
        assert!(t.contains_name(name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key);

        // Add another thing with the same keys.
        let thing2 = TestThing::new();
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing2);
        table_invariant(&t);

        // It has displaced the old thing.
        assert!(displaced.is_some());
        let displaced_item = &displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);

        // But it contains a thing with the same keys.
        assert!(t.contains_name(name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert a thing and then insert another thing with the same name.
    /// The previously inserted thing should be returned.
    fn insert_same_name() {
        let mut t: Table<PoolUuid, TestThing> = Table::default();
        table_invariant(&t);

        let uuid = PoolUuid::new_v4();
        let name = "name";
        let thing = TestThing::new();
        let thing_key = thing.stuff;

        // There was nothing in the table before, so displaced is empty.
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);
        assert_matches!(displaced, None);

        // t now contains thing.
        assert!(t.contains_name(name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let uuid2 = PoolUuid::new_v4();
        let thing2 = TestThing::new();
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name.to_owned()), uuid2, thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let displaced_item = &displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);
        assert_eq!(displaced_item[0].2.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_name(name));
        assert!(t.contains_uuid(uuid2));
        assert!(!t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid2).unwrap().1.stuff, thing_key2);
        assert_eq!(t.get_by_name(name).unwrap().1.stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert a thing and then insert another thing with the same uuid.
    /// The previously inserted thing should be returned.
    fn insert_same_uuid() {
        let mut t: Table<PoolUuid, TestThing> = Table::default();
        table_invariant(&t);

        let uuid = PoolUuid::new_v4();
        let name = "name";
        let thing = TestThing::new();
        let thing_key = thing.stuff;

        // There was nothing in the table before, so displaced is empty.
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);
        assert_matches!(displaced, None);

        // t now contains thing.
        assert!(t.contains_name(name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let name2 = "name2";
        let thing2 = TestThing::new();
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name2.to_owned()), uuid, thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let displaced_item = &displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);
        assert_eq!(displaced_item[0].2.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_uuid(uuid));
        assert!(t.contains_name(name2));
        assert!(!t.contains_name(name));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key2);
        assert_eq!(t.get_by_name(name2).unwrap().1.stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert two things, then insert a thing that matches one name and one
    /// uuid of each. Both existing things should be returned.
    fn insert_same_uuid_and_same_name() {
        let mut t: Table<PoolUuid, TestThing> = Table::default();
        table_invariant(&t);

        let uuid1 = PoolUuid::new_v4();
        let name1 = "name1";
        let thing1 = TestThing::new();
        let thing_key1 = thing1.stuff;

        let uuid2 = PoolUuid::new_v4();
        let name2 = "name2";
        let thing2 = TestThing::new();
        let thing_key2 = thing2.stuff;

        // Insert first item. There was nothing in the table before, so
        // displaced is empty.
        let displaced = t.insert(Name::new(name1.to_owned()), uuid1, thing1);
        table_invariant(&t);
        assert_matches!(displaced, None);

        // t now contains thing1.
        assert!(t.contains_name(name1));
        assert!(t.contains_uuid(uuid1));

        // Insert second item. No conflicts, so nothing is displaced.
        let displaced = t.insert(Name::new(name2.to_owned()), uuid2, thing2);
        table_invariant(&t);
        assert_matches!(displaced, None);

        // t now also contains thing2.
        assert!(t.contains_name(name2));
        assert!(t.contains_uuid(uuid2));

        // Create a third thing with the uuid of one and the name of the
        // other.
        let uuid3 = uuid1;
        let name3 = name2;
        let thing3 = TestThing::new();
        let thing_key3 = thing3.stuff;

        // Insert third item.
        let displaced = t.insert(Name::new(name3.to_owned()), uuid3, thing3);
        table_invariant(&t);

        // The items displaced consist of two items.
        assert!(displaced.is_some());
        let displaced_items = &displaced.unwrap();
        assert_eq!(displaced_items.len(), 2);

        // The first displaced item has the name of the just inserted item.
        assert_eq!(&*displaced_items[0].0, name3);
        assert_eq!(displaced_items[0].1, uuid2);
        assert_eq!(displaced_items[0].2.stuff, thing_key2);

        // The second displaced items has the uuid of the just inserted item.
        assert_eq!(&*displaced_items[1].0, name1);
        assert_eq!(displaced_items[1].1, uuid3);
        assert_eq!(displaced_items[1].2.stuff, thing_key1);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_uuid(uuid3));
        assert!(t.contains_name(name3));
        assert_eq!(t.get_by_uuid(uuid3).unwrap().1.stuff, thing_key3);
        assert_eq!(t.get_by_name(name3).unwrap().1.stuff, thing_key3);
        assert_eq!(t.len(), 1);
    }
}
