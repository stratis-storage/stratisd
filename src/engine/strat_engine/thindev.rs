// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.


use devicemapper::{DM, DevId, DeviceInfo, DmFlags};
use engine::EngineResult;
use engine::strat_engine::thinpooldev::ThinPoolDev;
use types::Sectors;

#[derive(Clone)]
pub struct ThinDev {
    name: String,
    pub dev_info: Option<DeviceInfo>,
    pub thin_id: u32,
    pub size: Option<Sectors>,
    dm_name: Option<String>,
    params: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum ThinStatus {
    Good(Sectors),
    Fail,
}

/// support use of DM to thin provisioned devices over pools
impl ThinDev {
    pub fn new(name: &str) -> ThinDev {
        ThinDev {
            name: name.to_owned(),
            dev_info: None,
            thin_id: 0,
            size: None,
            dm_name: None,
            params: None,
        }
    }

    /// Generate a Vec<> to be passed to DM.  The format of the Vec entries are:
    /// "<start> <length> thin </dev/mapper/poolname> <thin_id>"
    fn dm_table(&self,
                pool_dev_str: &String,
                thin_id: u32,
                length: &Sectors)
                -> Vec<(u64, u64, String, String)> {
        let mut table = Vec::new();

        let params = format!("{} {}", pool_dev_str, thin_id);

        table.push((0u64, length.0, "thin".to_owned(), params));
        debug!("dmtable line : {:?}", table);
        table

    }

    /// Use DM to create a thin provisioned DM device.
    pub fn setup(&mut self,
                 dm: &DM,
                 thin_pool: &mut ThinPoolDev,
                 thin_id: u32,
                 length: &Sectors)
                 -> EngineResult<()> {

        try!(thin_pool.message(dm, &format!("create_thin {}", thin_id)));
        try!(dm.device_create(&self.name, None, DmFlags::empty()));
        let table = self.dm_table(&thin_pool.dev_info.unwrap().device().dstr(),
                                  thin_id,
                                  length);
        let id = &DevId::Name(&self.name);
        self.dev_info = Some(try!(dm.table_load(id, &table)));
        try!(dm.device_suspend(id, DmFlags::empty()));
        Ok(())
    }

    pub fn teardown(&mut self, dm: &DM) -> EngineResult<()> {
        try!(dm.device_remove(&DevId::Name(&self.name), DmFlags::empty()));
        Ok(())
    }
}
