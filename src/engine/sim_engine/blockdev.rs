// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use engine::Dev;

use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::randomization::Randomizer;

#[derive(Debug, Eq, PartialEq)]
/// A list of very basic states a SimDev can be in.
pub enum State {
    OK,
    FAILED,
}

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    pub devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    pub state: State,
}

impl Dev for SimDev {}

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
