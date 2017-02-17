// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use devicemapper::{DM, DevId, DeviceInfo, DmFlags};
use engine::EngineResult;

use std::path::Path;

use super::blockdev::BlockDev;

use types::DataBlocks;
use types::Sectors;

pub struct ThinPoolDev {
    pub name: String,
    pub dev_info: Option<DeviceInfo>,
    data_block_size: Option<Sectors>,
    pub low_water_mark: Option<DataBlocks>,
    pub meta_dev: Option<BlockDev>,
    pub data_dev: Option<BlockDev>,
}

/// support use of DM to create pools for thin provisioned devices
impl ThinPoolDev {
    pub fn new(name: &str) -> ThinPoolDev {
        ThinPoolDev {
            name: name.to_owned(),
            dev_info: None,
            data_block_size: None,
            low_water_mark: None,
            meta_dev: None,
            data_dev: None,
        }
    }
    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries is:
    /// <start sec> <length> "thin-pool" /dev/meta /dev/data <block size> <low water mark>
    fn dm_table(&self,
                length: Sectors,
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

    /// Use DM to create a "thin-pool".  A "thin-pool" is shared space for
    /// other thin provisioned devices to use.
    ///
    /// See section "Setting up a fresh pool device":
    /// https://www.kernel.org/doc/Documentation/device-mapper/thin-provisioning.txt
    pub fn setup(&mut self,
                 dm: &DM,
                 length: Sectors,
                 data_block_size: Sectors,
                 low_water_mark: DataBlocks,
                 meta_dev: &Path,
                 data_dev: &Path)
                 -> EngineResult<()> {

        debug!("setup : {}", self.name);
        try!(dm.device_create(&self.name, None, DmFlags::empty()));

        let table = self.dm_table(length, data_block_size, low_water_mark, meta_dev, data_dev);
        self.data_block_size = Some(data_block_size);
        self.low_water_mark = Some(low_water_mark);
        let id = &DevId::Name(&self.name);
        self.dev_info = Some(try!(dm.table_load(id, &table)));
        try!(dm.device_suspend(id, DmFlags::empty()));

        Ok(())
    }

    pub fn message(&self, dm: &DM, message: &str) -> EngineResult<()> {
        try!(dm.target_msg(&DevId::Name(&self.name), 0, message));

        Ok(())
    }

    pub fn teardown(&mut self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));

        Ok(())
    }
}
