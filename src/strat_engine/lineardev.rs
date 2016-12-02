
use engine::EngineResult;

use super::blockdev::BlockDev;
use types::SectorOffset;
use uuid::Uuid;

use devicemapper::{DM, DeviceInfo, Device, DmFlags, DevId};


pub struct LinearDev {
    pub name: String,
    pub uuid: Uuid,
    pub device: Option<Device>,
    pub block_devs: Vec<BlockDev>,
}

impl LinearDev {
    pub fn new_lineardev(name: &str) -> LinearDev {
        LinearDev {
            name: name.to_owned(),
            uuid: Uuid::new_v4(),
            device: None,
            block_devs: Vec::new(),
        }
    }

    pub fn dm_table(&self, block_devs: Vec<BlockDev>) -> Vec<(u64, u64, String, String)> {
        let mut table = Vec::new();
        let mut offset = SectorOffset(0);
        for block_dev in block_devs {
            let (start, length) = block_dev.avail_range();
            let line =
                (*offset, length.0, "linear".to_owned(), format!("{} {}", block_dev.dstr(), start));
            table.push(line);
            offset = offset + SectorOffset(length.0);
        }

        table
    }


    pub fn concat(&mut self, name: &str, block_devs: Vec<BlockDev>) -> EngineResult<DeviceInfo> {
        let dm = DM::new().unwrap();
        let table = self.dm_table(block_devs);
        let id = &DevId::Name(name);

        try!(dm.device_create(&name, None, DmFlags::empty()));

        let di = try!(dm.table_load(id, &table));

        Ok((di))
    }
}
