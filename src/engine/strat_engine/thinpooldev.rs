// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{DM, DevId, DeviceInfo, DmFlags};
use engine::{EngineError, EngineResult, ErrorEnum};
use engine::strat_engine::lineardev::LinearDev;

use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use types::DataBlocks;
use types::Sectors;

pub struct ThinPoolDev {
    name: String,
    dev_info: DeviceInfo,
    meta_dev: LinearDev,
    data_dev: LinearDev,
}

impl fmt::Debug for ThinPoolDev {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.name())
    }
}


/// Use DM to create a "thin-pool".  A "thin-pool" is shared space for
/// other thin provisioned devices to use.
///
/// See section "Setting up a fresh pool device":
/// https://www.kernel.org/doc/Documentation/device-mapper/thin-provisioning.txt
impl ThinPoolDev {
    /// Construct a new ThinPoolDev with the given data and meta devs.  The
    /// ThinPoolDev is used as backing for by ThinDev.
    pub fn new(name: &str,
               dm: &DM,
               length: Sectors,
               data_block_size: Sectors,
               low_water_mark: DataBlocks,
               meta: LinearDev,
               data: LinearDev)
               -> EngineResult<ThinPoolDev> {
        try!(dm.device_create(&name, None, DmFlags::empty()));

        let meta_dev_path = try!(meta.path());
        let data_dev_path = try!(data.path());
        let table = ThinPoolDev::dm_table(length,
                                          data_block_size,
                                          low_water_mark,
                                          &meta_dev_path,
                                          &data_dev_path);
        let id = &DevId::Name(&name);
        let di = try!(dm.table_load(id, &table));
        try!(dm.device_suspend(id, DmFlags::empty()));

        Ok(ThinPoolDev {
            name: name.to_owned(),
            dev_info: di,
            meta_dev: meta,
            data_dev: data,
        })
    }

    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries is:
    /// <start sec> <length> "thin-pool" /dev/meta /dev/data <block size> <low water mark>
    fn dm_table(length: Sectors,
                data_block_size: Sectors,
                low_water_mark: DataBlocks,
                meta_dev: &Path,
                data_dev: &Path)
                -> Vec<(u64, u64, String, String)> {
        let mut table = Vec::new();
        let params = format!("{} {} {} {} 1 skip_block_zeroing",
                             meta_dev.to_string_lossy(),
                             data_dev.to_string_lossy(),
                             *data_block_size,
                             *low_water_mark);
        table.push((0u64, length.0, "thin-pool".to_owned(), params));
        debug!("dmtable line : {:?}", table);
        table
    }

    pub fn message(&self, dm: &DM, message: &str) -> EngineResult<()> {
        try!(dm.target_msg(&DevId::Name(&self.name), 0, message));

        Ok(())
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

    pub fn teardown(&mut self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));
        try!(self.data_dev.teardown(dm));
        try!(self.meta_dev.teardown(dm));
        Ok(())
    }
}
