// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::BTreeMap;

use time::Timespec;
use devicemapper::Device;

use types::{Sectors, SectorOffset};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MDA {
    pub last_updated: Timespec,
    length: u32,
    crc: u32,
    offset: SectorOffset,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDev {
    pub stratisdev_id: String,
    pub dev: Device,
    pub id: String,
    pub path: PathBuf,
    pub sectors: Sectors,
    pub mdaa: MDA,
    pub mdab: MDA, // Key is meta_dev dm name
}

#[derive(Debug, Clone)]
pub enum BlockMember {
    Present(Rc<RefCell<BlockDev>>),
}

impl BlockMember {
    pub fn present(&self) -> Option<Rc<RefCell<BlockDev>>> {
        match *self {
            BlockMember::Present(ref x) => Some(x.clone()),
        }
    }
}


impl BlockDev {}

#[derive(Debug, Clone)]
pub struct BlockDevs(pub BTreeMap<String, BlockMember>);

impl BlockDevs {}
