// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    any::type_name,
    collections::{hash_map, HashMap},
    fmt,
    iter::{FromIterator, IntoIterator},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::engine::{
    types::{AsUuid, Name},
    KeyActions, Pool,
};

/// Map UUID and name to T items.
pub struct Table<U, T> {
    name_to_uuid: HashMap<Name, U>,
    items: HashMap<U, (Name, T)>,
}

impl<U, T> fmt::Debug for Table<U, T>
where
    U: AsUuid,
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_map()
            .entries(
                self.iter()
                    .map(|(name, uuid, item)| ((name.to_string(), uuid), item)),
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

impl<U, T> FromIterator<(Name, U, T)> for Table<U, T>
where
    U: AsUuid,
{
    fn from_iter<I>(i: I) -> Self
    where
        I: IntoIterator<Item = (Name, U, T)>,
    {
        i.into_iter()
            .fold(Table::default(), |mut table, (name, uuid, t)| {
                table.insert(name, uuid, t);
                table
            })
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

/// Wrapper around RwLockReadGuard. This wrapper provides additional
/// debug logging.
pub struct LockableReadGuard<'a, T: ?Sized>(RwLockReadGuard<'a, T>);

impl<T> Deref for LockableReadGuard<'_, T>
where
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<T> Drop for LockableReadGuard<'_, T>
where
    T: ?Sized,
{
    fn drop(&mut self) {
        debug!("Read lock on {} dropped", type_name::<T>());
    }
}

/// Wrapper around RwLockWriteGuard. This wrapper provides additional
/// debug logging.
pub struct LockableWriteGuard<'a, T: ?Sized>(RwLockWriteGuard<'a, T>);

impl<T> Deref for LockableWriteGuard<'_, T>
where
    T: ?Sized,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<T> DerefMut for LockableWriteGuard<'_, T>
where
    T: ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl<T> Drop for LockableWriteGuard<'_, T>
where
    T: ?Sized,
{
    fn drop(&mut self) {
        debug!("Write lock on {} dropped", type_name::<T>());
    }
}

#[derive(Debug)]
pub struct Lockable<T: ?Sized> {
    lock: Arc<RwLock<T>>,
}

impl<T> Lockable<T>
where
    T: Send + Sync,
{
    pub fn new(inner: T) -> Self {
        Lockable {
            lock: Arc::new(RwLock::new(inner)),
        }
    }
}

impl<T> Lockable<T>
where
    T: ?Sized,
{
    pub async fn read(&self) -> LockableReadGuard<'_, T> {
        debug!("Acquiring read lock acquired on {}...", type_name::<T>());
        let guard = LockableReadGuard(self.lock.read().await);
        debug!("Read lock acquired on {}", type_name::<T>());
        guard
    }

    pub async fn write(&self) -> LockableWriteGuard<'_, T> {
        debug!("Acquiring write lock acquired on {}...", type_name::<T>());
        let guard = LockableWriteGuard(self.lock.write().await);
        debug!("Write lock acquired on {}", type_name::<T>());
        guard
    }
}

impl<T> Lockable<T>
where
    T: Pool + 'static,
{
    pub fn into_dyn_pool(self) -> Lockable<dyn Pool> {
        Lockable {
            lock: self.lock as Arc<RwLock<dyn Pool>>,
        }
    }
}

impl<T> Lockable<T>
where
    T: KeyActions + 'static,
{
    pub fn into_dyn_key_handler(self) -> Lockable<dyn KeyActions> {
        Lockable {
            lock: self.lock as Arc<RwLock<dyn KeyActions>>,
        }
    }
}

impl<T> Clone for Lockable<T>
where
    T: ?Sized,
{
    fn clone(&self) -> Self {
        Lockable {
            lock: Arc::clone(&self.lock),
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
