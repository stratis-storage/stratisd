// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::OpenOptions;
use std::path::PathBuf;

use time::Timespec;
use devicemapper::Device;

use types::{Sectors, SectorOffset};
use engine::{EngineResult, EngineError, ErrorEnum};

use consts::*;

use super::metadata::SigBlock;
use super::metadata::write_hdr_buf;

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

    pub fn write_sigblock(&self) -> EngineResult<()> {
        let mut buf = [0u8; SECTOR_SIZE as usize];
        self.sigblock.write(&mut buf, 0);
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        Ok(try!(write_hdr_buf(&buf, &mut f, &self.sigblock)))
    }

    pub fn wipe_sigblock(&mut self) -> EngineResult<()> {
        let buf = [0u8; SECTOR_SIZE as usize];
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        Ok(try!(write_hdr_buf(&buf, &mut f, &self.sigblock)))
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

    /// The available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (SectorOffset, Sectors) {
        let start = self.sigblock.main_bda_size();
        let length = self.sigblock.total_size - start - self.sigblock.aux_bda_size();
        (SectorOffset(*start), length)
    }
}
