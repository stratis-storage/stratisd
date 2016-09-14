// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Read, Write, ErrorKind, Seek, SeekFrom};
use std::fs::{OpenOptions, read_dir};
use std::path::{Path, PathBuf};
use std::io;
use std::rc::{Rc, Weak};
use std::cell::RefCell;
use std::str::{FromStr, from_utf8};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::cmp::min;

use nix::sys::stat;
use time::Timespec;
use devicemapper::{DM, Device};
use crc::crc32;
use byteorder::{LittleEndian, ByteOrder};
use uuid::Uuid;
use bytesize::ByteSize;

use types::{Sectors, SumSectors, SectorOffset, StratisResult, StratisError};
use consts::*;
use util::blkdev_size;
use dmdevice::DmDevice;


#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MDA {
    pub last_updated: Timespec,
    length: u32,
    crc: u32,
    offset: SectorOffset,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDev {
    pub Stratisdev_id: String,
    pub dev: Device,
    pub id: String,
    pub path: PathBuf,
    pub sectors: Sectors,
    pub mdaa: MDA,
    pub mdab: MDA,
    // Key is meta_dev dm name

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


impl BlockDev {







}

#[derive(Debug, Clone)]
pub struct BlockDevs(pub BTreeMap<String, BlockMember>);

impl BlockDevs {



}




