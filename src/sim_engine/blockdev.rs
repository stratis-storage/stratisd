// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use engine::Dev;

use std::path::Path;
use std::path::PathBuf;

use rand::Rng;
use rand::ThreadRng;
use rand::thread_rng;

#[derive(Clone, Debug, Eq, PartialEq)]
/// A list of very basic states a SimDev can be in.
pub enum State {
    OK,
    FAILED,
}

#[derive(Clone)]
/// A simulated device.
pub struct SimDev {
    pub name: PathBuf,
    rng: ThreadRng,
    pub state: State,
}

/// Implement Debug for SimDev explicitly as ThreadRng does not derive it.
/// See: https://github.com/rust-lang-nursery/rand/issues/118
impl fmt::Debug for SimDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{SimDev {:?} {:?}", self.name, self.state)
    }
}

impl fmt::Display for SimDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.get_id())
    }
}

impl Dev for SimDev {
    fn copy(&self) -> Box<Dev> {
        let simdev_copy = SimDev {
            name: self.name.clone(),
            rng: self.rng.clone(),
            state: self.state.clone(),
        };
        Box::new(simdev_copy)
    }

    fn get_id(&self) -> String {
        let id = self.name.to_str();

        match id {
            Some(x) => return String::from(x),
            None => return String::from("Conversion Failure"),
        }
    }

    fn has_same(&self, other: &Path) -> bool {
        self.get_id() == String::from(other.to_str().unwrap())
    }
}

impl SimDev {
    /// Generates a new device from any path.
    pub fn new_dev(name: &Path) -> Box<SimDev> {
        Box::new(SimDev {
            name: name.to_owned(),
            rng: thread_rng(),
            state: State::OK,
        })
    }

    /// Function that causes self to progress probabilistically to a new state.
    pub fn update(&mut self) {
        if self.rng.gen_weighted_bool(8) {
            self.state = State::FAILED;
        }
    }

    /// Checks usability of a SimDev
    pub fn usable(&self) -> bool {
        self.state == State::OK
    }
}
