// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{Write, Seek, SeekFrom};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use time::Timespec;
use devicemapper::Device;

use types::{Sectors, SectorOffset};
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::*;
use super::consts::*;

use super::metadata::SigBlock;

pub use super::BlockDevSave;

#[derive(Debug, Clone)]
pub struct BlockDev {
    pub dev: Device,
    pub devnode: PathBuf,
    pub sigblock: SigBlock,
}

impl BlockDev {
    pub fn to_save(&self) -> BlockDevSave {
        BlockDevSave {
            devnode: self.devnode.clone(),
            total_size: self.sigblock.total_size,
        }
    }

    /// Size of the BDA copy at the beginning of the blockdev
    fn main_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.sigblock.mda_sectors + self.sigblock.reserved_sectors) *
        SECTOR_SIZE
    }

    /// Size of the BDA copy at the end of the blockdev
    fn aux_bda_size(&self) -> u64 {
        *(BDA_STATIC_HDR_SIZE + self.sigblock.mda_sectors) * SECTOR_SIZE
    }

    pub fn write_sigblock(&self) -> EngineResult<()> {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        self.sigblock.write(&mut buf, 0);
        try!(self.write_hdr_buf(&self.devnode, &buf));
        Ok(())
    }

    pub fn wipe_sigblock(&mut self) -> EngineResult<()> {
        let buf = [0u8; SECTOR_SIZE as usize];
        try!(self.write_hdr_buf(&self.devnode, &buf));
        Ok(())
    }

    fn write_hdr_buf(&self, devnode: &Path, buf: &[u8; SECTOR_SIZE as usize]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(devnode));
        let zeroed = [0u8; (SECTOR_SIZE * 8) as usize];

        // Write 4K header to head & tail. Sigblock goes in sector 1.
        try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
        try!(f.write_all(buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 2) as usize..]));
        try!(f.seek(SeekFrom::End(-(self.aux_bda_size() as i64))));
        try!(f.write_all(&zeroed[..SECTOR_SIZE as usize]));
        try!(f.write_all(buf));
        try!(f.write_all(&zeroed[(SECTOR_SIZE * 2) as usize..]));
        try!(f.flush());

        Ok(())
    }

    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        try!(self.sigblock
            .mda
            .write_mdax(time, metadata)
            .map_err(|e| EngineError::Stratis(ErrorEnum::Error(e))));
        try!(self.write_sigblock());
        Ok(())
    }

    /// Get the "x:y" device string for this blockdev
    pub fn dstr(&self) -> String {
        self.dev.dstr()
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (SectorOffset, Sectors) {
        let start = SectorOffset(*BDA_STATIC_HDR_SIZE + *self.sigblock.mda_sectors +
                                 *self.sigblock.reserved_sectors);
        let length = Sectors(*self.sigblock.total_size - *start - *BDA_STATIC_HDR_SIZE -
                             *self.sigblock.mda_sectors);
        (start, length)
    }
}
