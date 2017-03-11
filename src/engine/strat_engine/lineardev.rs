// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{DM, DevId, DeviceInfo, DmFlags};

use engine::{EngineError, EngineResult, ErrorEnum};

use std::fmt;
use std::fs::File;
use std::path::PathBuf;

use super::blockdev::BlockDev;
use types::Sectors;

pub struct LinearDev {
    name: String,
    dev_info: DeviceInfo,
}

impl fmt::Debug for LinearDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.name())
    }
}

/// Use DM to concatenate a set of blockdevs together into a
/// /dev/mapper/xxx block device of continuous sectors.
impl LinearDev {
    /// Construct a new block device by concatenating the given block_devs
    /// into linear space.  Use DM to reserve enough space for the stratis
    /// metadata on each BlockDev.
    pub fn new(name: &str, dm: &DM, block_devs: &Vec<&BlockDev>) -> EngineResult<LinearDev> {

        try!(dm.device_create(&name, None, DmFlags::empty()));
        let table = LinearDev::dm_table(block_devs);
        let id = &DevId::Name(&name);
        let dev_info = try!(dm.table_load(id, &table));
        try!(dm.device_suspend(id, DmFlags::empty()));

        Ok(LinearDev {
            name: name.to_owned(),
            dev_info: dev_info,
        })
    }

    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries is:
    /// <logical start sec> <length> "linear" /dev/xxx <start offset>
    fn dm_table(block_devs: &Vec<&BlockDev>) -> Vec<(u64, u64, String, String)> {
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

    pub fn name(&self) -> &str {
        self.dev_info.name().clone()
    }

    pub fn path(&self) -> EngineResult<PathBuf> {
        match self.dev_info.device().path() {
            Some(path) => return Ok(path),
            None => {
                return Err(EngineError::Engine(ErrorEnum::Invalid,
                                               "No path associated with dev_info".into()))
            }
        }
    }

    pub fn teardown(&self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));

        Ok(())
    }
}
