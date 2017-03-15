// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::slice::IterMut;

use uuid::Uuid;

use super::engine::{HasName, HasUuid};


/// Map UUID and name to T items.
#[derive(Debug)]
pub struct Table<T: HasUuid> {
    items: Vec<(T, String)>,
    name_map: HashMap<String, usize>,
    uuid_map: HashMap<Uuid, usize>,
}

/// All operations are O(1).
/// The UUID is considered to be a constant value, but the name key
/// may change.
impl<T: HasName + HasUuid> Table<T> {
    pub fn new() -> Self {
        Table {
            items: Vec::new(),
            name_map: HashMap::new(),
            uuid_map: HashMap::new(),
        }

    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns true if map has an item corresponding to this name, else false.
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_map.contains_key(name)
    }

    /// Returns true if map has an item corresponding to this uuid, else false.
    #[allow(dead_code)]
    pub fn contains_uuid(&self, uuid: &Uuid) -> bool {
        self.uuid_map.contains_key(uuid)
    }

    /// Get item by name.
    pub fn get_by_name(&self, name: &str) -> Option<&T> {
        self.name_map.get(name).map(|index| &self.items[*index].0)
    }

    /// Get item by uuid.
    pub fn get_by_uuid(&self, uuid: &Uuid) -> Option<&T> {
        self.uuid_map.get(uuid).map(|index| &self.items[*index].0)
    }

    /// Get the name of the item.
    pub fn get_name(&self, uuid: &Uuid) -> Option<&str> {
        self.uuid_map.get(uuid).map(|index| &self.items[*index].1 as &str)
    }

    /// Get mutable item by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<&mut T> {
        if let Some(index) = self.name_map.get(name) {
            Some(&mut self.items[*index].0)
        } else {
            None
        }
    }

    /// Get mutable item by uuid.
    pub fn get_mut_by_uuid(&mut self, uuid: &Uuid) -> Option<&mut T> {
        if let Some(index) = self.uuid_map.get(uuid) {
            Some(&mut self.items[*index].0)
        } else {
            None
        }
    }

    /// Removes the Pool corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<T> {
        if let Some(index) = self.name_map.remove(name) {
            // There is guaranteed to be a last because there is at least
            // one index into the items.

            // Insert mappings for the about-to-be swapped element
            {
                let &(last_item, last_name) = self.items.last().unwrap();
                self.name_map.insert(last_name.into(), index);
                self.uuid_map.insert(last_item.uuid().clone(), index);
            }

            // Remove the item we want to remove and also the uuid mapping
            let (item, item_name) = self.items.swap_remove(index);
            self.uuid_map.remove(item.uuid());

            // Remove the name again, in case there is only one item.
            self.name_map.remove(name);

            Some(item)
        } else {
            None
        }
    }

    /// Removes the Pool corresponding to the uuid if there is one.
    pub fn remove_by_uuid(&mut self, uuid: &Uuid) -> Option<T> {
        if let Some(index) = self.uuid_map.remove(uuid) {
            // There is guaranteed to be a last because there is at least
            // one index into the items.

            // Insert mappings for the about-to-be swapped element
            {
                let &(last_item, last_name) = self.items.last().unwrap();
                self.name_map.insert(last_name.into(), index);
                self.uuid_map.insert(last_item.uuid().clone(), index);
            }

            // Remove the item we want to remove and also the uuid mapping
            let (item, item_name) = self.items.swap_remove(index);
            self.name_map.remove(&item_name);

            // Remove the uuid again, in case there is only one item.
            self.uuid_map.remove(uuid);

            Some(item)
        } else {
            None
        }
    }

    /// Inserts an item for given uuid and name.
    /// Returns a list of the items displaced, which may be empty if no items
    /// are displaced, have one entry if the uuid and the name map to the same
    /// item, and may have two entries if the uuid and the name map to
    /// different items.
    pub fn insert(&mut self, item: T, name: &str) -> Vec<(T, String)> {
        let name_item = self.remove_by_name(name);
        let uuid_item = self.remove_by_uuid(item.uuid());

        let future_last_index = self.items.len();
        self.name_map.insert(name.into(), future_last_index);
        self.uuid_map.insert(item.uuid().clone(), future_last_index);

        self.items.push(item);

        match (name_item, uuid_item) {
            (None, None) => vec![],
            (None, Some(item)) => vec![item],
            (Some(item), None) => vec![item],
            (Some(p1), Some(p2)) => vec![p1, p2],
        }
    }
}

#[cfg(test)]
mod tests {

    use rand;
    use uuid::Uuid;

    use super::super::engine::{HasName, HasUuid};

    use super::Table;

    #[derive(Debug)]
    struct TestThing {
        name: String,
        uuid: Uuid,
        stuff: u32,
    }

    // A global invariant checker for the table.
    // Verifies proper relationship between internal data structures.
    fn table_invariant<T>(table: &Table<T>) -> ()
        where T: HasName + HasUuid
    {
        let ref items = table.items;
        let ref name_map = table.name_map;
        let ref uuid_map = table.uuid_map;
        for i in 0..items.len() {
            let name = items[i].name();
            let uuid = items[i].uuid();
            assert!(name_map.get(name).unwrap() == &i);
            assert!(uuid_map.get(uuid).unwrap() == &i);
        }

        for name in name_map.keys() {
            let index = name_map.get(name).unwrap();
            assert!(items[*index].name() == name);
        }

        for uuid in uuid_map.keys() {
            let index = uuid_map.get(uuid).unwrap();
            assert!(items[*index].uuid() == uuid);
        }

    }

    impl TestThing {
        pub fn new(name: &str, uuid: &Uuid) -> TestThing {
            TestThing {
                name: name.to_owned(),
                uuid: uuid.clone(),
                stuff: rand::random::<u32>(),
            }
        }
    }

    impl HasUuid for TestThing {
        fn uuid(&self) -> &Uuid {
            &self.uuid
        }
    }

    impl HasName for TestThing {
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[test]
    /// Remove a test object by its uuid.
    /// Mutate the removed test object.
    /// Verify that the table is now empty and that removing by name yields
    /// no result.
    fn remove_existing_item() {
        let mut t: Table<TestThing> = Table::new();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        t.insert(TestThing::new(&name, &uuid));
        table_invariant(&t);

        assert!(t.get_by_name(&name).is_some());
        assert!(t.get_by_uuid(&uuid).is_some());
        let thing = t.remove_by_uuid(&uuid);
        table_invariant(&t);
        assert!(thing.is_some());
        let mut thing = thing.unwrap();
        thing.stuff = 0;
        assert!(t.is_empty());
        assert!(t.remove_by_name(&name).is_none());
        table_invariant(&t);

        assert!(t.get_by_name(&name).is_none());
        assert!(t.get_by_uuid(&uuid).is_none());
    }

    #[test]
    /// Insert a thing and then insert another thing with same keys.
    /// The previously inserted thing should be returned.
    /// You can't insert the identical thing, because that would be a move.
    /// This is good, because then you can't have a thing that is both in
    /// the table and not in the table.
    fn insert_same_keys() {
        let mut t: Table<TestThing> = Table::new();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        let thing = TestThing::new(&name, &uuid);
        let thing_key = thing.stuff;
        let displaced = t.insert(thing);
        table_invariant(&t);

        // There was nothing previously, so displaced must be empty.
        assert!(displaced.is_empty());

        // t now contains the inserted thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(&uuid));
        assert!(t.get_by_uuid(&uuid).unwrap().stuff == thing_key);

        // Add another thing with the same keys.
        let thing2 = TestThing::new(&name, &uuid);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(thing2);
        table_invariant(&t);

        // It has displaced the old thing.
        assert!(displaced.len() == 1);
        let ref displaced_item = displaced[0];
        assert!(displaced_item.name() == name);
        assert!(displaced_item.uuid() == &uuid);

        // But it contains a thing with the same keys.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(&uuid));
        assert!(t.get_by_uuid(&uuid).unwrap().stuff == thing_key2);
        assert!(t.len() == 1);
    }

    #[test]
    /// Insert a thing and then insert another thing with the same name.
    /// The previously inserted thing should be returned.
    fn insert_same_name() {
        let mut t: Table<TestThing> = Table::new();
        table_invariant(&t);

        let uuid = Uuid::new_v4();
        let name = "name";
        let thing = TestThing::new(&name, &uuid);
        let thing_key = thing.stuff;

        // There was nothing in the table before, so displaced is empty.
        let displaced = t.insert(thing);
        table_invariant(&t);
        assert!(displaced.is_empty());

        // t now contains thing.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(&uuid));

        // Insert new item with different UUID.
        let uuid2 = Uuid::new_v4();
        let thing2 = TestThing::new(&name, &uuid2);
        let thing_key2 = thing2.stuff;
        let displaced = t.insert(thing2);
        table_invariant(&t);

        // The items displaced consist exactly of the first item.
        assert!(displaced.len() == 1);
        let ref displaced_item = displaced[0];
        assert!(displaced_item.name() == name);
        assert!(displaced_item.uuid() == &uuid);
        assert!(displaced_item.stuff == thing_key);

        // The table contains the new item and has no memory of the old.
        assert!(t.contains_name(&name));
        assert!(t.contains_uuid(&uuid2));
        assert!(!t.contains_uuid(&uuid));
        assert!(t.get_by_uuid(&uuid2).unwrap().stuff == thing_key2);
        assert!(t.get_by_name(&name).unwrap().stuff == thing_key2);
        assert!(t.len() == 1);
    }
}
