// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::{hash_map, HashMap};
use std::iter::IntoIterator;

use uuid::Uuid;

use engine::Name;

/// Map UUID and name to T items.
#[derive(Debug)]
pub struct Table<T: HasName + HasUuid> {
    name_to_uuid: HashMap<Name, Uuid>,
    items: HashMap<Uuid, (Name, T)>,
}


impl<T: HasName + HasUuid> Default for Table<T> {
    fn default() -> Table<T> {
        Table {
            name_to_uuid: HashMap::default(),
            items: HashMap::default(),
        }
    }
}

pub struct Iter<'a, T: 'a> {
    items: hash_map::Iter<'a, Uuid, (Name, T)>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.items.next().map(|(_, &(_, ref item))| item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

pub struct IterMut<'a, T: 'a> {
    items: hash_map::IterMut<'a, Uuid, (Name, T)>,
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    #[inline]
    fn next(&mut self) -> Option<&'a mut T> {
        self.items
            .next()
            .map(|(_, &mut (_, ref mut item))| item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

pub struct IntoIter<T> {
    items: hash_map::IntoIter<Uuid, (Name, T)>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.items.next().map(|(_, (_, item))| item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

impl<T> IntoIterator for Table<T>
    where T: HasName + HasUuid
{
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        self.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Table<T>
    where T: HasName + HasUuid
{
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Table<T>
    where T: HasName + HasUuid
{
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> IterMut<'a, T> {
        self.iter_mut()
    }
}

/// All operations are O(1), although Name lookups are slightly disadvantaged
/// vs. Uuid lookups. In order to rename a T item, it must be removed,
/// renamed, and reinserted under the new name.
impl<T> Table<T>
    where T: HasName + HasUuid
{
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn iter(&self) -> Iter<T> {
        Iter { items: self.items.iter() }
    }

    pub fn iter_mut(&mut self) -> IterMut<T> {
        IterMut { items: self.items.iter_mut() }
    }

    pub fn into_iter(self) -> IntoIter<T> {
        IntoIter { items: self.items.into_iter() }
    }

    /// Returns true if map has an item corresponding to this name, else false.
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_to_uuid.contains_key(name)
    }

    /// Returns true if map has an item corresponding to this uuid, else false.
    pub fn contains_uuid(&self, uuid: Uuid) -> bool {
        self.items.contains_key(&uuid)
    }

    /// Get item by name.
    pub fn get_by_name(&self, name: &str) -> Option<&T> {
        self.name_to_uuid
            .get(&*name)
            .and_then(|uuid| self.items.get(uuid).map(|&(_, ref item)| item))
    }

    /// Get item by uuid.
    pub fn get_by_uuid(&self, uuid: Uuid) -> Option<&T> {
        self.items.get(&uuid).map(|&(_, ref item)| item)
    }

    /// Get mutable item by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<&mut T> {
        let uuid = match self.name_to_uuid.get(name) {
            Some(uuid) => *uuid,
            None => return None,
        };
        self.items
            .get_mut(&uuid)
            .map(|&mut (_, ref mut item)| item)
    }

    /// Get mutable item by uuid.
    pub fn get_mut_by_uuid(&mut self, uuid: Uuid) -> Option<&mut T> {
        self.items
            .get_mut(&uuid)
            .map(|&mut (_, ref mut item)| item)
    }

    /// Removes the item corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<T> {
        if let Some(uuid) = self.name_to_uuid.remove(name) {
            self.items.remove(&uuid).map(|(_, item)| item)
        } else {
            None
        }
    }

    /// Removes the item corresponding to the uuid if there is one.
    pub fn remove_by_uuid(&mut self, uuid: Uuid) -> Option<T> {
        if let Some((_, item)) = self.items.remove(&uuid) {
            let name = self.name_to_uuid
                .iter()
                .find(|&(_, item_uuid)| *item_uuid == uuid)
                .expect("should be there")
                .0
                .to_owned();
            self.name_to_uuid.remove(&*name);
            Some(item)
        } else {
            None
        }
    }

    /// Inserts an item for given uuid and name.
    /// Possibly returns the item displaced.
    pub fn insert(&mut self, item: T) -> Option<T> {
        match self.name_to_uuid.insert(item.name(), item.uuid()) {
            Some(old_uuid) => {
                // (existing name, _)
                match self.items.insert(item.uuid(), (item.name(), item)) {
                    // (existing name, existing uuid)
                    Some((_, old_item)) => Some(old_item),
                    // (existing name, new uuid)
                    None => {
                        let (_, old_item) = self.items.remove(&old_uuid).expect("should be there");
                        Some(old_item)
                    }
                }
            }
            None => {
                // (new name, existing uuid)
                if let Some((old_name, old_item)) =
                    self.items.insert(item.uuid(), (item.name(), item)) {
                    self.name_to_uuid
                        .remove(&old_name)
                        .expect("should be there");
                    Some(old_item)
                } else {
                    // (new name, new uuid)
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use rand;
    use uuid::Uuid;

    use super::super::engine::{HasName, HasUuid};

    use super::{Name, Table};

    #[derive(Debug)]
    struct TestThing {
        name: Name,
        uuid: Uuid,
        stuff: u32,
    }

    // A global invariant checker for the table.
    // Verifies proper relationship between internal data structures.
    fn table_invariant<T>(table: &Table<T>) -> ()
        where T: HasName + HasUuid
    {
        for (uuid, &(ref name, _)) in &table.items {
            assert_eq!(*uuid, *table.name_to_uuid.get(name).unwrap())
        }

        // No extra garbage
        assert_eq!(table.name_to_uuid.len(), table.items.len())
    }

    impl TestThing {
        pub fn new(name: &str, uuid: Uuid) -> TestThing {
            TestThing {
                name: Name::new(name.to_owned()),
                uuid: uuid.clone(),
                stuff: rand::random::<u32>(),
            }
        }
    }

    impl HasUuid for TestThing {
        fn uuid(&self) -> Uuid {
            self.uuid
        }
    }

    impl HasName for TestThing {
        fn name(&self) -> Name {
            self.name.clone()
        }
    }

    #[test]
    /// Remove a test object by its uuid.
    /// Mutate the removed test object.
    /// Verify that the table is now empty and that removing by name yields
    /// no result.
    fn remove_existing_item() {
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        t.insert(TestThing::new(&name, uuid));
        table_invariant(&t);

        assert!(t.get_by_name(&name).is_some());
        assert!(t.get_by_uuid(uuid).is_some());
        let thing = t.remove_by_uuid(uuid);
        table_invariant(&t);
        assert!(thing.is_some());
        let mut thing = thing.unwrap();
        thing.stuff = 0;
        assert!(t.is_empty());
        assert!(t.remove_by_name(&name).is_none());
        table_invariant(&t);

        assert!(t.get_by_name(&name).is_none());
        assert!(t.get_by_uuid(uuid).is_none());
    }

    #[test]
    /// Insert a thing and then insert another thing with same keys.
    /// The previously inserted thing should be returned.
    /// You can't insert the identical thing, because that would be a move.
    /// This is good, because then you can't have a thing that is both in
    /// the table and not in the table.
    fn insert_same_keys() {
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        let thing = TestThing::new(&name, uuid);
        let thing_key = thing.stuff;
        let displaced = t.insert(thing);
        table_invariant(&t);

        // There was nothing previously, so displaced must be empty.
        assert!(displaced.is_none());

        // t now contains the inserted thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().stuff, thing_key);

        // Add another thing with the same keys.
        let thing2 = TestThing::new(&name, uuid);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(thing2);
        table_invariant(&t);

        // It has displaced the old thing.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item.name(), name);
        assert_eq!(displaced_item.uuid(), uuid);

        // But it contains a thing with the same keys.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert a thing and then insert another thing with the same name.
    /// The previously inserted thing should be returned.
    fn insert_same_name() {
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        let thing = TestThing::new(&name, uuid);
        let thing_key = thing.stuff;

        // There was nothing in the table before, so displaced is empty.
        let displaced = t.insert(thing);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now contains thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let uuid2 = Uuid::new_v4();
        let thing2 = TestThing::new(&name, uuid2);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item.name(), name);
        assert_eq!(displaced_item.uuid(), uuid);
        assert_eq!(displaced_item.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid2));
        assert!(!t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid2).unwrap().stuff, thing_key2);
        assert_eq!(t.get_by_name(&name).unwrap().stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert a thing and then insert another thing with the same uuid.
    /// The previously inserted thing should be returned.
    fn insert_same_uuid() {
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        let thing = TestThing::new(&name, uuid);
        let thing_key = thing.stuff;

        // There was nothing in the table before, so displaced is empty.
        let displaced = t.insert(thing);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now contains thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let name2 = "name2";
        let thing2 = TestThing::new(&name2, uuid);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item.name(), name);
        assert_eq!(displaced_item.uuid(), uuid);
        assert_eq!(displaced_item.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_uuid(uuid));
        assert!(t.contains_name(name2));
        assert!(!t.contains_name(name));
        assert_eq!(t.get_by_uuid(uuid).unwrap().stuff, thing_key2);
        assert_eq!(t.get_by_name(&name2).unwrap().stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }
}
