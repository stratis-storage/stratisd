// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use engine::Cache;

use super::randomization::Randomizer;

#[derive(Clone, Debug, Eq, PartialEq)]
/// A list of very basic states a SimCacheDev can be in.
pub enum CacheState {
    OK,
    FAILED,
}

#[derive(Clone, Debug)]
/// A simulated cache device.
pub struct SimCacheDev {
    pub name: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    pub state: CacheState,
}

impl SimCacheDev {
    /// Generates a new cache device from a path.
    pub fn new_cache(rdm: Rc<RefCell<Randomizer>>, name: &Path) -> SimCacheDev {
        SimCacheDev {
            name: name.to_owned(),
            rdm: rdm,
            state: CacheState::OK,
        }
    }
    pub fn update(&mut self) {
        if self.rdm.borrow_mut().throw_die() {
            self.state = CacheState::FAILED;
        }
    }

    /// Checks usability of a SimCacheDev
    pub fn usable(&self) -> bool {
        self.state == CacheState::OK
    }
}

impl Cache for SimCacheDev {
    fn copy(&self) -> Box<Cache> {
        let cache_copy = SimCacheDev {
            name: self.name.clone(),
            rdm: self.rdm.clone(),
            state: self.state.clone(),
        };
        Box::new(cache_copy)
    }

    fn get_id(&self) -> String {
        let id = self.name.to_str();

        match id {
            Some(x) => return String::from(x),
            None => return String::from("Conversion Failure"),
        }
    }
    fn has_same(&self, other: &Path) -> bool {
        self.name == other
    }
}
