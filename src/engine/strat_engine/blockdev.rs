// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::fs::OpenOptions;
use std::path::PathBuf;

use devicemapper::Device;
use devicemapper::Segment;
use devicemapper::Sectors;
use time::Timespec;

use super::super::errors::EngineResult;
use super::super::types::{DevUuid, PoolUuid};

use super::metadata::BDA;
use super::range_alloc::RangeAllocator;


#[derive(Debug)]
pub struct BlockDev {
    dev: Device,
    pub devnode: PathBuf,
    bda: BDA,
    used: RangeAllocator,
}

impl BlockDev {
    pub fn new(dev: Device, devnode: PathBuf, bda: BDA, allocator: RangeAllocator) -> BlockDev {
        BlockDev {
            dev: dev,
            devnode: devnode,
            bda: bda,
            used: allocator,
        }
    }

    pub fn wipe_metadata(self) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        BDA::wipe(&mut f)
    }

    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        let mut f = try!(OpenOptions::new().write(true).open(&self.devnode));
        self.bda.save_state(time, metadata, &mut f)
    }

    pub fn load_state(&self) -> EngineResult<Option<Vec<u8>>> {
        let mut f = try!(OpenOptions::new().read(true).open(&self.devnode));
        self.bda.load_state(&mut f)
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> Segment {
        let start = self.bda.size();
        let size = self.size();
        // Blockdev size is at least MIN_DEV_SIZE, so this can fail only if
        // size of metadata area exceeds 1 GiB. Initial metadata area size
        // is 4 MiB.
        assert!(start <= size);
        Segment::new(self.dev, start, size - start)
    }

    /// The device's UUID.
    pub fn uuid(&self) -> &DevUuid {
        self.bda.dev_uuid()
    }

    /// The device's pool's UUID.
    pub fn pool_uuid(&self) -> &PoolUuid {
        self.bda.pool_uuid()
    }

    /// The device's size.
    pub fn size(&self) -> Sectors {
        self.bda.dev_size()
    }

    /// Last time metadata was written to this device.
    pub fn last_update_time(&self) -> Option<&Timespec> {
        self.bda.last_update_time()
    }

    pub fn available(&self) -> Sectors {
        self.used.available()
    }

    // Find some sector ranges that could be allocated. If more
    // sectors are needed than our capacity, return partial results.
    pub fn request_space(&mut self, size: Sectors) -> (Sectors, Vec<Segment>) {
        let (size, segs) = self.used.request(size);
        (size,
         segs.iter()
             .map(|&(start, len)| Segment::new(self.dev, start, len))
             .collect())
    }
}
