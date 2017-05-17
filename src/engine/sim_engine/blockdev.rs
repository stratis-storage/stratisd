// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::super::engine::Dev;

use super::randomization::Randomizer;

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    pub devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
}

impl Dev for SimDev {}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(rdm: Rc<RefCell<Randomizer>>, devnode: &Path) -> SimDev {
        SimDev {
            devnode: devnode.to_owned(),
            rdm: rdm,
        }
    }
}
