// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Borrow;
use std::fs::OpenOptions;
use std::io;
use std::io::{Write, ErrorKind};

use devicemapper::{DM, Device, DevId, DmFlags, DM_SUSPEND, TargetLine};

use types::{StratisResult, StratisError};
use util::blkdev_size;
use consts::*;

#[derive(Debug, Clone, PartialEq)]
pub struct DmDevice {
    pub dm_name: String,
    pub dev: Device,
}

impl DmDevice {
    pub fn new<T1, T2>(dm: &DM, name: &str, table: &[(u64, u64, T1, T2)])
                  -> StratisResult<DmDevice>
        where T1: Borrow<str>,
              T2: Borrow<str>, {
        let id = &DevId::Name(name);

        let di = match dm.device_status(id) {
            Ok(di) => {
                dbgp!("Found {}", name);
                di
            },
            Err(_) => {
                try!(dm.device_create(&name, None, DmFlags::empty()));
                let di = try!(dm.table_load(id, table));
                try!(dm.device_suspend(id, DmFlags::empty()));

                dbgp!("Created {}", name);
                di
            }
        };

        Ok(DmDevice {
            dm_name: name.to_owned(),
            dev: di.device(),
        })
    }

    pub fn dstr(&self) -> String {
        format!("{}:{}", self.dev.major, self.dev.minor)
    }

    pub fn reload<T1, T2>(&self, dm: &DM, table: &[(u64, u64, T1, T2)])
                          -> StratisResult<()>
        where T1: Borrow<str>,
              T2: Borrow<str>, {
        let id = &DevId::Name(&self.dm_name);

        try!(dm.table_load(id, table));
        try!(dm.device_suspend(id, DM_SUSPEND));
        try!(dm.device_suspend(id, DmFlags::empty()));

        Ok(())
    }

    pub fn suspend(&self, dm: &DM) -> StratisResult<()> {
        try!(dm.device_suspend(&DevId::Name(&self.dm_name), DM_SUSPEND));

        Ok(())
    }

    pub fn unsuspend(&self, dm: &DM) -> StratisResult<()> {
        try!(dm.device_suspend(&DevId::Name(&self.dm_name), DmFlags::empty()));

        Ok(())
    }

    pub fn table_load<T1, T2>(&self, dm: &DM, table: &[(u64, u64, T1, T2)])
                              -> StratisResult<()>
        where T1: Borrow<str>,
              T2: Borrow<str>, {
        try!(dm.table_load(&DevId::Name(&self.dm_name), table));

        Ok(())
    }

    pub fn teardown(&self, dm: &DM) -> StratisResult<()> {
        dbgp!("tearing down {}", self.dm_name);
        try!(dm.device_remove(&DevId::Name(&self.dm_name), DmFlags::empty()));

        Ok(())
    }

    pub fn clear(&self) -> StratisResult<()> {


        Ok(())
    }

    pub fn table_status(&self, dm: &DM) -> StratisResult<Vec<TargetLine>> {
        let (_, status) = try!(
            dm.table_status(&DevId::Name(&self.dm_name), DmFlags::empty()));

        Ok(status)
    }

    pub fn message(&self, dm: &DM, message: &str) -> StratisResult<()> {
        try!(dm.target_msg(&DevId::Name(&self.dm_name), 0, message));

        Ok(())
    }
}
