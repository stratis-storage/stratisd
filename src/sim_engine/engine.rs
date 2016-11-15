// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;

use engine::Pool;

use std::cell::RefCell;
use std::path::Path;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::iter::FromIterator;
use std::rc::Rc;

use super::blockdev::SimDev;
use super::pool::SimPool;
use super::randomization::Randomizer;



#[derive(Debug)]
pub struct SimEngine {
    pub pools: BTreeMap<String, SimPool>,
    rdm: Rc<RefCell<Randomizer>>,
}

impl SimEngine {
    pub fn new() -> SimEngine {
        SimEngine {
            pools: BTreeMap::new(),
            rdm: Rc::new(RefCell::new(Randomizer::new())),
        }
    }
}

impl Engine for SimEngine {
    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let mut devs: Vec<SimDev> =
            blockdev_paths.iter().map(|x| SimDev::new_dev(self.rdm.clone(), x)).collect();

        for dev in devs.iter_mut() {
            dev.update();
        }

        let bad_devs: Vec<&SimDev> = devs.iter().filter(|dev| !dev.usable()).collect();

        if !bad_devs.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("some devices are unusable".into())));
        }

        let pool = SimPool::new_pool(self.rdm.clone(), devs.as_slice(), raid_level);

        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Error("X".into())));
        }

        self.pools.insert(name.to_owned(), pool);
        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        let entry = match self.pools.entry(name.into()) {
            Entry::Vacant(_) => return Ok(()),
            Entry::Occupied(entry) => entry,
        };
        if !entry.get().filesystems.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("filesystems remaining on pool"
                .into())));
        };
        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("could not free devices in pool"
                .into())));
        };
        entry.remove();
        Ok(())
    }
    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool> {

        let return_pool = match self.pools.get_mut(name) {
            Some(pool) => pool,
            None => return Err(EngineError::Stratis(ErrorEnum::NotFound(name.into()))),
        };

        Ok(return_pool)
    }

    fn pools(&mut self) -> BTreeMap<&str, &mut Pool> {

        BTreeMap::from_iter(self.pools.iter_mut().map(|x| (x.0 as &str, x.1 as &mut Pool)))

    }

    fn configure_simulator(&mut self, denominator: u32) -> EngineResult<()> {
        self.rdm.borrow_mut().set_probability(denominator);
        Ok(())
    }
}
