// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use devicemapper::{Bytes, IEC, Sectors};

use super::super::engine::{BlockDev, HasUuid};
use super::super::types::{BlockDevState, DevUuid};

use super::randomization::Randomizer;

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    uuid: Uuid,
    user_info: Option<String>,
    hardware_info: Option<String>,
    initialization_time: u64,
}

impl BlockDev for SimDev {
    fn devnode(&self) -> PathBuf {
        self.devnode.clone()
    }

    fn user_info(&self) -> Option<&str> {
        self.user_info.as_ref().map(|x| &**x)
    }

    fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }

    fn hardware_info(&self) -> Option<&str> {
        self.hardware_info.as_ref().map(|x| &**x)
    }

    fn initialization_time(&self) -> DateTime<Utc> {
        Utc.timestamp(self.initialization_time as i64, 0)
    }

    fn total_size(&self) -> Sectors {
        Bytes(IEC::Gi).sectors()
    }

    fn state(&self) -> BlockDevState {
        BlockDevState::InUse
    }
}

impl HasUuid for SimDev {
    fn uuid(&self) -> DevUuid {
        self.uuid
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(rdm: Rc<RefCell<Randomizer>>, devnode: &Path) -> SimDev {
        SimDev {
            devnode: devnode.to_owned(),
            rdm,
            uuid: Uuid::new_v4(),
            user_info: None,
            hardware_info: None,
            initialization_time: Utc::now().timestamp() as u64,
        }
    }
}
