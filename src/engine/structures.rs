// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;
use std::fmt::Debug;
use std::slice::IterMut;

use uuid::Uuid;

pub trait PoolTableValue: Debug {
    fn uuid(&self) -> &Uuid;
    fn name(&self) -> &str;
}

/// Map various keys to pool objects
#[derive(Debug)]
pub struct PoolTable<Pool: PoolTableValue> {
    pools: Vec<Pool>,
    name_map: HashMap<String, usize>,
    uuid_map: HashMap<Uuid, usize>,
}

impl<Pool: PoolTableValue> PoolTable<Pool> {
    pub fn new() -> PoolTable<Pool> {
        PoolTable {
            pools: Vec::new(),
            name_map: HashMap::new(),
            uuid_map: HashMap::new(),
        }

    }
    /// Returns true if map has a Pool corresponding to this name, else false.
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_map.contains_key(name)
    }

    /// Returns true if map has a Pool corresponding to this uuid, else false.
    #[allow(dead_code)]
    pub fn contains_uuid(&self, uuid: &Uuid) -> bool {
        self.uuid_map.contains_key(uuid)
    }

    /// Get pool by name.
    pub fn get_by_name(&self, name: &str) -> Option<&Pool> {
        self.name_map.get(name).map(|index| &self.pools[*index])
    }

    /// Get pool by uuid.
    #[allow(dead_code)]
    pub fn get_by_uuid(&self, uuid: &Uuid) -> Option<&Pool> {
        self.uuid_map.get(uuid).map(|index| &self.pools[*index])
    }

    /// Get mutable pool by name.
    pub fn get_mut_by_name(&mut self, name: &str) -> Option<&mut Pool> {
        if let Some(index) = self.name_map.get(name) {
            Some(&mut self.pools[*index])
        } else {
            None
        }
    }

    /// Get mutable pool by uuid.
    #[allow(dead_code)]
    pub fn get_mut_by_uuid(&mut self, uuid: &Uuid) -> Option<&mut Pool> {
        if let Some(index) = self.uuid_map.get(uuid) {
            Some(&mut self.pools[*index])
        } else {
            None
        }
    }

    /// A mutable iterator through Pools.
    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> IterMut<Pool> {
        self.pools.iter_mut()
    }

    /// Removes the Pool corresponding to name if there is one.
    pub fn remove_by_name(&mut self, name: &str) -> Option<Pool> {
        if let Some(index) = self.name_map.remove(name) {
            // There is guaranteed to be a last because there is at least
            // one index into the pool.

            // Insert mappings for the about-to-be swapped element
            {
                let last_pool = &self.pools.last().unwrap();
                self.name_map.insert(last_pool.name().into(), index);
                self.uuid_map.insert(last_pool.uuid().clone(), index);
            }

            // Remove the pool we want to remove and also the uuid mapping
            let pool = self.pools.swap_remove(index);
            self.uuid_map.remove(pool.uuid());

            // Remove the name again, in case there is only one pool.
            self.name_map.remove(name);

            Some(pool)
        } else {
            None
        }
    }

    /// Removes the Pool corresponding to the uuid if there is one.
    pub fn remove_by_uuid(&mut self, uuid: &Uuid) -> Option<Pool> {
        if let Some(index) = self.uuid_map.remove(uuid) {
            // There is guaranteed to be a last because there is at least
            // one index into the pool.

            // Insert mappings for the about-to-be swapped element
            {
                let last_pool = &self.pools.last().unwrap();
                self.name_map.insert(last_pool.name().into(), index);
                self.uuid_map.insert(last_pool.uuid().clone(), index);
            }

            // Remove the pool we want to remove and also the uuid mapping
            let pool = self.pools.swap_remove(index);
            self.name_map.remove(pool.name());

            // Remove the uuid again, in case there is only one pool.
            self.uuid_map.remove(uuid);

            Some(pool)
        } else {
            None
        }
    }

    /// Inserts a Pool for given uuid and name.
    /// Returns a list of the pools displaced, which may be empty if no pools
    /// are displaced, have one entry if the uuid and the name map to the same
    /// pool, and may have two entries if the uuid and the name map to
    /// different pools.
    pub fn insert(&mut self, pool: Pool) -> Vec<Pool> {
        let name_pool = self.remove_by_name(pool.name());
        let uuid_pool = self.remove_by_uuid(pool.uuid());

        let future_last_index = self.pools.len();
        self.name_map.insert(pool.name().into(), future_last_index);
        self.uuid_map.insert(pool.uuid().clone(), future_last_index);

        self.pools.push(pool);

        match (name_pool, uuid_pool) {
            (None, None) => vec![],
            (None, Some(pool)) => vec![pool],
            (Some(pool), None) => vec![pool],
            (Some(p1), Some(p2)) => vec![p1, p2],
        }
    }
}
