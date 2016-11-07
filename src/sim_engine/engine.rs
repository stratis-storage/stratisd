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
use std::iter::FromIterator;
use std::rc::Rc;

use super::blockdev::SimDev;
use super::pool::SimPool;
use super::randomization::Randomizer;



#[derive(Debug)]
pub struct SimEngine {
    pub pools: BTreeMap<String, Box<Pool>>,
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

        let devs: Vec<Box<SimDev>> =
            blockdev_paths.iter().map(|x| SimDev::new_dev(self.rdm.clone(), x)).collect();

        match self.rdm.borrow_mut().get_bad_item(&devs) {
            Some(d) => {
                let path_as_str = d.name.to_str().unwrap_or("unstringable path");
                return Err(EngineError::Stratis(ErrorEnum::Busy(path_as_str.into())));
            }
            None => {}
        }

        let pool = SimPool::new_pool(self.rdm.clone(), devs.as_slice(), raid_level);

        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Error("X".into())));
        }

        self.pools.insert(name.to_owned(), pool);
        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        if self.rdm.borrow_mut().throw_die() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("X".into())));
        }

        self.pools.remove(name);

        Ok(())
    }
    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Box<Pool>> {

        let return_pool = match self.pools.get_mut(name) {
            Some(pool) => pool,
            None => return Err(EngineError::Stratis(ErrorEnum::NotFound(name.into()))),
        };

        Ok(return_pool)
    }

    fn list_pools(&self) -> EngineResult<BTreeMap<String, Box<Pool>>> {

        Ok(BTreeMap::from_iter(self.pools.iter().map(|x| (x.0.clone(), x.1.copy()))))

    }
}
