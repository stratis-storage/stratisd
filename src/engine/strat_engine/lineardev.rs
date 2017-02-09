// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{DM, DevId, DeviceInfo, DmFlags};

use engine::{EngineError, EngineResult, ErrorEnum};
use super::blockdev::BlockDev;
use types::Sectors;

pub struct LinearDev {
    name: String,
    dev_info: Option<DeviceInfo>,
}

/// support use of DM to concatenate blockdevs and creat e a
/// /dev/mapper/<name> for use as continuous sectors.
impl LinearDev {
    pub fn new(name: &str) -> LinearDev {
        LinearDev {
            name: name.to_owned(),
            dev_info: None,
        }
    }

    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries is:
    /// <logical start sec> <length> "linear" /dev/xxx <start offset>
    fn dm_table(&self, block_devs: &Vec<&BlockDev>) -> Vec<(u64, u64, String, String)> {
        let mut table = Vec::new();
        let mut logical_start_sector = Sectors(0);
        for block_dev in block_devs {
            let (start, length) = block_dev.avail_range();
            let dstr = block_dev.dstr();
            let line = (*logical_start_sector,
                        *length,
                        "linear".to_owned(),
                        format!("{} {}", dstr, *start));
            debug!("dmtable line : {:?}", line);
            table.push(line);
            logical_start_sector = logical_start_sector + length;
        }

        table
    }

    /// Use DM to concatenate a set of blockdevs together into a
    /// /dev/mapper/xxx block device of continuous sectors.
    pub fn concat(&mut self, dm: &DM, block_devs: &Vec<&BlockDev>) -> EngineResult<()> {

        try!(dm.device_create(&self.name, None, DmFlags::empty()));
        let table = self.dm_table(block_devs);
        let id = &DevId::Name(&self.name);
        self.dev_info = Some(try!(dm.table_load(id, &table)));

        try!(dm.device_suspend(id, DmFlags::empty()));

        return Ok(());
    }

    pub fn name(&self) -> EngineResult<&str> {
        match self.dev_info {
            Some(ref di) => return Ok(di.name().clone()),
            None => return Err(EngineError::Engine(ErrorEnum::Invalid, "No dev_info".into())),
        }
    }

    pub fn teardown(&self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));

        Ok(())
    }
}
