// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use devicemapper::{Bytes, Sectors, IEC};

use crate::engine::{
    engine::BlockDev, sim_engine::randomization::Randomizer, types::MaybeDbusPath,
};

#[derive(Debug)]
/// A simulated device.
pub struct SimDev {
    devnode: PathBuf,
    rdm: Rc<RefCell<Randomizer>>,
    user_info: Option<String>,
    hardware_info: Option<String>,
    initialization_time: u64,
    dbus_path: MaybeDbusPath,
}

impl BlockDev for SimDev {
    fn devnode(&self) -> PathBuf {
        self.devnode.clone()
    }

    fn user_info(&self) -> Option<&str> {
        self.user_info.as_ref().map(|x| &**x)
    }

    fn hardware_info(&self) -> Option<&str> {
        self.hardware_info.as_ref().map(|x| &**x)
    }

    fn initialization_time(&self) -> DateTime<Utc> {
        Utc.timestamp(self.initialization_time as i64, 0)
    }

    fn size(&self) -> Sectors {
        Bytes(IEC::Gi).sectors()
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }
}

impl SimDev {
    /// Generates a new device from any devnode.
    pub fn new(
        rdm: Rc<RefCell<Randomizer>>,
        devnode: &Path,
        _key_desc: Option<&str>,
    ) -> (Uuid, SimDev) {
        (
            Uuid::new_v4(),
            SimDev {
                devnode: devnode.to_owned(),
                rdm,
                user_info: None,
                hardware_info: None,
                initialization_time: Utc::now().timestamp() as u64,
                dbus_path: MaybeDbusPath(None),
            },
        )
    }

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }
}
