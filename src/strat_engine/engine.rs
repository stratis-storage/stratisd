// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::str::FromStr;
use std::iter::FromIterator;
use std::io;
use std::io::ErrorKind;

use devicemapper::Device;

use engine::Engine;
use engine::EngineError;
use engine::EngineResult;
use engine::ErrorEnum;
use engine::Pool;

use super::pool::StratPool;


#[derive(Debug)]
pub struct StratEngine {
    pub pools: BTreeMap<String, StratPool>,
}

impl StratEngine {
    pub fn new() -> StratEngine {
        StratEngine { pools: BTreeMap::new() }
    }
}

impl Engine for StratEngine {
    fn configure_simulator(&mut self, _denominator: u32) -> EngineResult<()> {
        Ok(()) // we're not the simulator and not configurable, so just say ok
    }

    fn create_pool(&mut self,
                   name: &str,
                   blockdev_paths: &[&Path],
                   raid_level: u16)
                   -> EngineResult<()> {

        if self.pools.contains_key(name) {
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists(name.into())));
        }

        let mut devices = BTreeSet::new();
        for path in blockdev_paths {
            let dev = try!(Device::from_str(&path.to_string_lossy()));
            devices.insert(dev);
        }

        if devices.len() != blockdev_paths.len() {
            return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                      "duplicate blockdevs")));
        }

        for (_, pool) in &self.pools {
            for (_, bd) in &pool.block_devs {
                if devices.contains(&bd.dev) {
                    return Err(EngineError::Io(io::Error::new(ErrorKind::InvalidInput,
                                                              format!("blockdev {} already \
                                                                       used in pool {}",
                                                                      bd.dstr(),
                                                                      pool.name))));
                }
            }
        }

        let pool = try!(StratPool::new(name, devices, raid_level, true));

        self.pools.insert(name.to_owned(), pool);
        Ok(())
    }

    fn destroy_pool(&mut self, name: &str) -> EngineResult<()> {
        self.pools.remove(name);

        Ok(())
    }

    fn get_pool(&mut self, name: &str) -> EngineResult<&mut Pool> {
        match self.pools.get_mut(name) {
            Some(pool) => Ok(pool),
            None => Err(EngineError::Stratis(ErrorEnum::NotFound(name.into()))),
        }
    }

    fn pools(&mut self) -> BTreeMap<&str, &mut Pool> {
        BTreeMap::from_iter(self.pools.iter_mut().map(|x| (x.0 as &str, x.1 as &mut Pool)))
    }
}
