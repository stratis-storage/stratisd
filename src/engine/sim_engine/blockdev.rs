// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use engine::Dev;

use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::randomization::Randomizer;

#[derive(Clone, Debug, Eq, PartialEq)]
/// A list of very basic states a SimDev can be in.
pub enum State {
    OK,
    FAILED,
}

#[derive(Clone,Debug)]
/// A simulated device.
pub struct SimDev {
    pub devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    pub state: State,
}

impl fmt::Display for SimDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.get_id())
    }
}

impl Dev for SimDev {
    fn get_id(&self) -> String {
        let id = self.devnode.to_str();

        match id {
            Some(x) => return String::from(x),
            None => return String::from("Conversion Failure"),
        }
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(rdm: Rc<RefCell<Randomizer>>, devnode: &Path) -> SimDev {
        SimDev {
            devnode: devnode.to_owned(),
            rdm: rdm,
            state: State::OK,
        }
    }

    /// Function that causes self to progress probabilistically to a new state.
    pub fn update(&mut self) {
        if self.rdm.borrow_mut().throw_die() {
            self.state = State::FAILED;
        }
    }

    /// Checks usability of a SimDev
    pub fn usable(&self) -> bool {
        self.state == State::OK
    }
}
