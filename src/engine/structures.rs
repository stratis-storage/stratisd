// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::slice::IterMut;

use uuid::Uuid;

use super::engine::{HasName, HasUuid};


/// Map UUID and name to T items.
#[derive(Debug)]
pub struct Table<T: HasName + HasUuid> {
    items: Vec<T>,
    name_map: HashMap<String, usize>,
    uuid_map: HashMap<Uuid, usize>,
}

/// All operations are O(1).
/// The implementation does not priviledge the name key over the UUID key
/// in any way. They are both treated as constants once the item has been
/// inserted. In order to rename a T item, it must be removed, renamed, and
/// reinserted under the new name.
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
        self.name_map.get(name).map(|index| &self.items[*index])
    }

    /// Get item by uuid.
    pub fn get_by_uuid(&self, uuid: &Uuid) -> Option<&T> {
        self.uuid_map.get(uuid).map(|index| &self.items[*index])
    }

    /// Get mutable item by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<&mut T> {
        if let Some(index) = self.name_map.get(name) {
            Some(&mut self.items[*index])
        } else {
            None
        }
    }

    /// Get mutable item by uuid.
    pub fn get_mut_by_uuid(&mut self, uuid: &Uuid) -> Option<&mut T> {
        if let Some(index) = self.uuid_map.get(uuid) {
            Some(&mut self.items[*index])
        } else {
            None
        }
    }

    /// A mutable iterator through Pools.
    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> IterMut<T> {
        self.items.iter_mut()
    }

    /// Removes the Pool corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<T> {
        if let Some(index) = self.name_map.remove(name) {
            // There is guaranteed to be a last because there is at least
            // one index into the items.

            // Insert mappings for the about-to-be swapped element
            {
                let last_item = &self.items.last().unwrap();
                self.name_map.insert(last_item.name().into(), index);
                self.uuid_map.insert(last_item.uuid().clone(), index);
            }

            // Remove the item we want to remove and also the uuid mapping
            let item = self.items.swap_remove(index);
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
                let last_item = &self.items.last().unwrap();
                self.name_map.insert(last_item.name().into(), index);
                self.uuid_map.insert(last_item.uuid().clone(), index);
            }

            // Remove the item we want to remove and also the uuid mapping
            let item = self.items.swap_remove(index);
            self.name_map.remove(item.name());

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
    pub fn insert(&mut self, item: T) -> Vec<T> {
        let name_item = self.remove_by_name(item.name());
        let uuid_item = self.remove_by_uuid(item.uuid());

        let future_last_index = self.items.len();
        self.name_map.insert(item.name().into(), future_last_index);
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
