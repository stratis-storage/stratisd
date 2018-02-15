// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::{HashMap, hash_map};
use std::iter::IntoIterator;

use uuid::Uuid;

use engine::Name;

/// Map UUID and name to T items.
#[derive(Debug)]
pub struct Table<T> {
    name_to_uuid: HashMap<Name, Uuid>,
    items: HashMap<Uuid, (Name, T)>,
}


impl<T> Default for Table<T> {
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
    type Item = (&'a Name, &'a Uuid, &'a T);

    #[inline]
    fn next(&mut self) -> Option<(&'a Name, &'a Uuid, &'a T)> {
        self.items
            .next()
            .map(|(uuid, &(ref name, ref item))| (&*name, uuid, item))
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
    type Item = (&'a Name, &'a Uuid, &'a mut T);

    #[inline]
    fn next(&mut self) -> Option<(&'a Name, &'a Uuid, &'a mut T)> {
        self.items
            .next()
            .map(|(uuid, &mut (ref name, ref mut item))| (&*name, uuid, item))
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
    type Item = (Name, Uuid, T);

    #[inline]
    fn next(&mut self) -> Option<(Name, Uuid, T)> {
        self.items
            .next()
            .map(|(uuid, (name, item))| (name, uuid, item))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.items.size_hint()
    }
}

impl<T> IntoIterator for Table<T> {
    type Item = (Name, Uuid, T);
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        self.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Table<T> {
    type Item = (&'a Name, &'a Uuid, &'a T);
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Table<T> {
    type Item = (&'a Name, &'a Uuid, &'a mut T);
    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> IterMut<'a, T> {
        self.iter_mut()
    }
}

/// All operations are O(1), although Name lookups are slightly disadvantaged
/// vs. Uuid lookups. In order to rename a T item, it must be removed,
/// renamed, and reinserted under the new name.
impl<T> Table<T> {
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
    pub fn get_by_name(&self, name: &str) -> Option<(Uuid, &T)> {
        self.name_to_uuid
            .get(&*name)
            .and_then(|uuid| self.items.get(uuid).map(|&(_, ref item)| (*uuid, item)))
    }

    /// Get item by uuid.
    pub fn get_by_uuid(&self, uuid: Uuid) -> Option<(Name, &T)> {
        self.items
            .get(&uuid)
            .map(|&(ref name, ref item)| (name.clone(), item))
    }

    /// Get mutable item by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<(Uuid, &mut T)> {
        let uuid = match self.name_to_uuid.get(name) {
            Some(uuid) => *uuid,
            None => return None,
        };
        self.items
            .get_mut(&uuid)
            .map(|&mut (_, ref mut item)| (uuid, item))
    }

    /// Get mutable item by uuid.
    pub fn get_mut_by_uuid(&mut self, uuid: Uuid) -> Option<(Name, &mut T)> {
        self.items
            .get_mut(&uuid)
            .map(|&mut (ref name, ref mut item)| (name.clone(), item))
    }

    /// Removes the item corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<(Uuid, T)> {
        if let Some(uuid) = self.name_to_uuid.remove(name) {
            self.items.remove(&uuid).map(|(_, item)| (uuid, item))
        } else {
            None
        }
    }

    /// Removes the item corresponding to the uuid if there is one.
    pub fn remove_by_uuid(&mut self, uuid: Uuid) -> Option<(Name, T)> {
        if let Some((name, item)) = self.items.remove(&uuid) {
            self.name_to_uuid.remove(&name);
            Some((name, item))
        } else {
            None
        }
    }

    /// Inserts an item for given uuid and name.
    /// Possibly returns the items displaced.
    /// If two items are displaced, the one displaced by matching name is
    /// returned first.
    pub fn insert(&mut self, name: Name, uuid: Uuid, item: T) -> Option<Vec<(Name, Uuid, T)>> {
        let old_uuid = self.name_to_uuid.insert(name.clone(), uuid);
        let old_pair = self.items.insert(uuid, (name.clone(), item));
        match (old_uuid, old_pair) {
            // Two possibilities: One entry with same name and uuid ejected OR
            // two entries, one with same name and different uuid and one with
            // same uuid and different name.
            (Some(old_uuid), Some((old_name, old_item))) => {
                Some(if old_uuid == uuid {
                         assert_eq!(old_name, name);
                         vec![(name, uuid, old_item)]
                     } else {
                         assert!(old_name != name);
                         let other_uuid = self.name_to_uuid
                             .remove(&old_name)
                             .expect("invariant requires existence");
                         let (other_name, other_item) = self.items
                             .remove(&old_uuid)
                             .expect("invariant requires existence");
                         assert_eq!(other_name, name);
                         assert_eq!(other_uuid, uuid);
                         vec![(name, old_uuid, other_item), (old_name, uuid, old_item)]
                     })
            }
            // entry with same name but different uuid ejected
            (Some(old_uuid), None) => {
                let (other_name, other_item) = self.items
                    .remove(&old_uuid)
                    .expect("invariant requires existence");
                assert_eq!(other_name, name);
                Some(vec![(name, old_uuid, other_item)])
            }
            // entry with same uuid but different name ejected
            (None, Some((old_name, old_item))) => {
                let other_uuid = self.name_to_uuid
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

#[cfg(test)]
mod tests {

    use rand;
    use uuid::Uuid;

    use engine::Name;

    use super::Table;

    #[derive(Debug)]
    struct TestThing {
        name: String,
        uuid: Uuid,
        stuff: u32,
    }

    // A global invariant checker for the table.
    // Verifies proper relationship between internal data structures.
    fn table_invariant<T>(table: &Table<T>) -> () {
        for (uuid, &(ref name, _)) in &table.items {
            assert_eq!(*uuid, *table.name_to_uuid.get(name).unwrap())
        }

        for (name, uuid) in &table.name_to_uuid {
            assert_eq!(*name, table.items.get(uuid).unwrap().0);
        }

        // No extra garbage
        assert_eq!(table.name_to_uuid.len(), table.items.len())
    }

    impl TestThing {
        pub fn new(name: &str, uuid: Uuid) -> TestThing {
            TestThing {
                name: name.to_owned(),
                uuid: uuid.clone(),
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
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        t.insert(Name::new(name.to_owned()),
                 uuid,
                 TestThing::new(&name, uuid));
        table_invariant(&t);

        assert!(t.get_by_name(&name).is_some());
        assert!(t.get_by_uuid(uuid).is_some());
        let thing = t.remove_by_uuid(uuid);
        table_invariant(&t);
        assert!(thing.is_some());
        let mut thing = thing.unwrap();
        thing.1.stuff = 0;
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
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);

        // There was nothing previously, so displaced must be empty.
        assert!(displaced.is_none());

        // t now contains the inserted thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key);

        // Add another thing with the same keys.
        let thing2 = TestThing::new(&name, uuid);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing2);
        table_invariant(&t);

        // It has displaced the old thing.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);

        // But it contains a thing with the same keys.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key2);
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
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now contains thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let uuid2 = Uuid::new_v4();
        let thing2 = TestThing::new(&name, uuid2);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name.to_owned()), uuid2, thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);
        assert_eq!(displaced_item[0].2.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid2));
        assert!(!t.contains_uuid(uuid));
        assert_eq!(t.get_by_uuid(uuid2).unwrap().1.stuff, thing_key2);
        assert_eq!(t.get_by_name(&name).unwrap().1.stuff, thing_key2);
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
        let displaced = t.insert(Name::new(name.to_owned()), uuid, thing);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now contains thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(uuid));

        // Insert new item with different UUID.
        let name2 = "name2";
        let thing2 = TestThing::new(&name2, uuid);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(Name::new(name2.to_owned()), uuid, thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.is_some());
        let ref displaced_item = displaced.unwrap();
        assert_eq!(&*displaced_item[0].0, name);
        assert_eq!(displaced_item[0].1, uuid);
        assert_eq!(displaced_item[0].2.stuff, thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_uuid(uuid));
        assert!(t.contains_name(name2));
        assert!(!t.contains_name(name));
        assert_eq!(t.get_by_uuid(uuid).unwrap().1.stuff, thing_key2);
        assert_eq!(t.get_by_name(&name2).unwrap().1.stuff, thing_key2);
        assert_eq!(t.len(), 1);
    }

    #[test]
    /// Insert two things, then insert a thing that matches one name and one
    /// uuid of each. Both existing things should be returned.
    fn insert_same_uuid_and_same_name() {
        let mut t: Table<TestThing> = Table::default();
        table_invariant(&t);

        let uuid1 = Uuid::new_v4();
        let name1 = "name1";
        let thing1 = TestThing::new(&name1, uuid1);
        let thing_key1 = thing1.stuff;

        let uuid2 = Uuid::new_v4();
        let name2 = "name2";
        let thing2 = TestThing::new(&name2, uuid2);
        let thing_key2 = thing2.stuff;

        // Insert first item. There was nothing in the table before, so
        // displaced is empty.
        let displaced = t.insert(Name::new(name1.to_owned()), uuid1, thing1);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now contains thing1.
        assert!(t.contains_name(&name1));
        assert!(t.contains_uuid(uuid1));

        // Insert second item. No conflicts, so nothing is displaced.
        let displaced = t.insert(Name::new(name2.to_owned()), uuid2, thing2);
        table_invariant(&t);
        assert!(displaced.is_none());

        // t now also contains thing2.
        assert!(t.contains_name(&name2));
        assert!(t.contains_uuid(uuid2));

        // Create a third thing with the uuid of one and the name of the
        // other.
        let uuid3 = uuid1;
        let name3 = name2;
        let thing3 = TestThing::new(&name3, uuid3);
        let thing_key3 = thing3.stuff;

        // Insert third item.
        let displaced = t.insert(Name::new(name3.to_owned()), uuid3, thing3);
        table_invariant(&t);

        // The items displaced consist of two items.
        assert!(displaced.is_some());
        let ref displaced_items = displaced.unwrap();
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
        assert_eq!(t.get_by_name(&name3).unwrap().1.stuff, thing_key3);
        assert_eq!(t.len(), 1);
    }
}
