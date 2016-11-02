// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use std::path::Path;
use std::path::PathBuf;

use engine::Cache;

#[derive(Clone, Debug, Eq, PartialEq)]
/// A list of very basic states a SimCacheDev can be in.
pub enum CacheState {
    OK,
    FAILED,
}

#[derive(Clone)]
/// A simulated cache device.
pub struct SimCacheDev {
    pub name: PathBuf,
    pub state: CacheState,
}

impl SimCacheDev {
    /// Generates a new cache device from a path.
    pub fn new_cache(name: &Path) -> Box<SimCacheDev> {
        Box::new(SimCacheDev {
            name: name.to_owned(),
            state: CacheState::OK,
        })
    }
    pub fn update(&mut self) {}

    /// Checks usability of a SimCacheDev
    pub fn usable(&self) -> bool {
        self.state == CacheState::OK
    }
}

impl fmt::Debug for SimCacheDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{SimDev {:?} {:?}", self.name, self.state)
    }
}

impl Cache for SimCacheDev {
    fn copy(&self) -> Box<Cache> {
        let cache_copy = SimCacheDev {
            name: self.name.clone(),
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
}
