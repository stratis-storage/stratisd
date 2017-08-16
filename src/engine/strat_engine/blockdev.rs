// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::fs::OpenOptions;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use devicemapper::{Device, Sectors};

use super::super::errors::EngineResult;
use super::super::types::{DevUuid, PoolUuid};

use super::metadata::BDA;
use super::range_alloc::RangeAllocator;
use super::serde_structs::{BlockDevSave, Recordable};


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

    /// Returns the blockdev's Device
    pub fn device(&self) -> &Device {
        &self.dev
    }

    pub fn wipe_metadata(&self) -> EngineResult<()> {
        let mut f = OpenOptions::new().write(true).open(&self.devnode)?;
        BDA::wipe(&mut f)
    }

    pub fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> EngineResult<()> {
        let mut f = OpenOptions::new().write(true).open(&self.devnode)?;
        self.bda.save_state(time, metadata, &mut f)
    }

    /// List the available-for-upper-layer-use range in this blockdev.
    pub fn avail_range(&self) -> (Sectors, Sectors) {
        let start = self.metadata_size();
        let size = self.current_capacity();
        // Blockdev size is at least MIN_DEV_SIZE, so this can fail only if
        // size of metadata area exceeds 1 GiB. Initial metadata area size
        // is 4 MiB.
        assert!(start <= size);
        (start, size - start)
    }

    /// The device's UUID.
    pub fn uuid(&self) -> &DevUuid {
        self.bda.dev_uuid()
    }

    /// The device's pool's UUID.
    pub fn pool_uuid(&self) -> &PoolUuid {
        self.bda.pool_uuid()
    }

    /// Last time metadata was written to this device.
    pub fn last_update_time(&self) -> Option<&DateTime<Utc>> {
        self.bda.last_update_time()
    }

    // Find some sector ranges that could be allocated. If more
    // sectors are needed than our capacity, return partial results.
    pub fn request_space(&mut self, size: Sectors) -> (Sectors, Vec<(Sectors, Sectors)>) {
        self.used.request(size)
    }

    // ALL SIZE METHODS

    /// The size of the device as recorded in the metadata.
    pub fn recorded_size(&self) -> Sectors {
        self.bda.dev_size()
    }

    /// The actual size of the device now.
    pub fn current_capacity(&self) -> Sectors {
        self.used.capacity()
    }

    /// The number of Sectors on this device used by Stratis for metadata
    pub fn metadata_size(&self) -> Sectors {
        self.bda.size()
    }

    /// The number of Sectors on this device not allocated for any purpose.
    /// self.current_capacity() - self.metadata_size() >= self.available()
    pub fn available(&self) -> Sectors {
        self.used.available()
    }

    /// The maximum size of variable length metadata that can be accommodated.
    /// self.max_metadata_size() < self.metadata_size()
    pub fn max_metadata_size(&self) -> Sectors {
        self.bda.max_data_size()
    }
}

impl Recordable<BlockDevSave> for BlockDev {
    fn record(&self) -> BlockDevSave {
        BlockDevSave { devnode: self.devnode.clone() }
    }
}
