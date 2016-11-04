// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use std::path::Path;
use std::path::PathBuf;

use engine::Cache;

use rand::Rng;
use rand::ThreadRng;
use rand::thread_rng;

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
    rng: ThreadRng,
    pub state: CacheState,
}

/// Implement Debug for SimDev explicitly as ThreadRng does not derive it.
/// See: https://github.com/rust-lang-nursery/rand/issues/118
impl fmt::Debug for SimCacheDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{SimDev {:?} {:?}", self.name, self.state)
    }
}

impl SimCacheDev {
    /// Generates a new cache device from a path.
    pub fn new_cache(name: &Path) -> Box<SimCacheDev> {
        Box::new(SimCacheDev {
            name: name.to_owned(),
            rng: thread_rng(),
            state: CacheState::OK,
        })
    }
    pub fn update(&mut self) {
        if self.rng.gen_weighted_bool(8) {
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
            rng: self.rng.clone(),
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
