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
    pub devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    pub state: CacheState,
}

impl SimCacheDev {
    /// Generates a new cache device from a path.
    pub fn new(rdm: Rc<RefCell<Randomizer>>, devnode: &Path) -> SimCacheDev {
        SimCacheDev {
            devnode: devnode.to_owned(),
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
    fn get_id(&self) -> String {
        let id = self.devnode.to_str();

        match id {
            Some(x) => return String::from(x),
            None => return String::from("Conversion Failure"),
        }
    }
}
