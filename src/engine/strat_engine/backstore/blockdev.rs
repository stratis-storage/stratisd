// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

#[cfg(feature = "dbus_enabled")]
use dbus;

use std::fs::OpenOptions;
use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};

use devicemapper::{Device, Sectors};

use stratis::StratisResult;

use super::super::super::engine::BlockDev;
use super::super::super::types::{BlockDevState, DevUuid, PoolUuid};

use super::super::serde_structs::{BlockDevSave, Recordable};

use super::metadata::BDA;
use super::range_alloc::RangeAllocator;

#[derive(Debug)]
pub struct StratBlockDev {
    dev: Device,
    pub(super) devnode: PathBuf,
    bda: BDA,
    used: RangeAllocator,
    user_info: Option<String>,
    hardware_info: Option<String>,
    #[cfg(feature = "dbus_enabled")]
    user_data: Option<dbus::Path<'static>>,
}

impl StratBlockDev {
    /// Make a new BlockDev from the parameters.
    /// Allocate space for the Stratis metadata on the device.
    /// - dev: the device, identified by number
    /// - devnode: the device node
    /// - bda: the device's BDA
    /// - other_segments: segments claimed for non-Stratis metadata use
    /// - user_info: user settable identifying information
    /// - hardware_info: identifying information in the hardware
    /// Returns an error if it is impossible to allocate all segments on the
    /// device.
    /// NOTE: It is possible that the actual device size is greater than
    /// the recorded device size. In that case, the additional space available
    /// on the device is simply invisible to the blockdev. Consequently, it
    /// is invisible to the engine, and is not part of the total size value
    /// reported on the D-Bus.
    pub fn new(
        dev: Device,
        devnode: PathBuf,
        bda: BDA,
        upper_segments: &[(Sectors, Sectors)],
        user_info: Option<String>,
        hardware_info: Option<String>,
    ) -> StratisResult<StratBlockDev> {
        let mut segments = vec![(Sectors(0), bda.size())];
        segments.extend(upper_segments);
        let allocator = RangeAllocator::new(bda.dev_size(), &segments)?;

        Ok(StratBlockDev {
            dev,
            devnode,
            bda,
            used: allocator,
            user_info,
            hardware_info,
            #[cfg(feature = "dbus_enabled")]
            user_data: None,
        })
    }

    /// Returns the blockdev's Device
    pub fn device(&self) -> &Device {
        &self.dev
    }

    pub fn wipe_metadata(&self) -> StratisResult<()> {
        let mut f = OpenOptions::new().write(true).open(&self.devnode)?;
        BDA::wipe(&mut f)
    }

    pub fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()> {
        let mut f = OpenOptions::new().write(true).open(&self.devnode)?;
        self.bda.save_state(time, metadata, &mut f)
    }

    /// The device's UUID.
    pub fn uuid(&self) -> DevUuid {
        self.bda.dev_uuid()
    }

    /// The device's pool's UUID.
    #[allow(dead_code)]
    pub fn pool_uuid(&self) -> PoolUuid {
        self.bda.pool_uuid()
    }

    /// Last time metadata was written to this device.
    #[allow(dead_code)]
    pub fn last_update_time(&self) -> Option<&DateTime<Utc>> {
        self.bda.last_update_time()
    }

    /// Find some sector ranges that could be allocated. If more
    /// sectors are needed than our capacity, return partial results.
    /// If all sectors are desired, use available() method to get all.
    pub fn request_space(&mut self, size: Sectors) -> (Sectors, Vec<(Sectors, Sectors)>) {
        self.used.request(size)
    }

    // ALL SIZE METHODS
    /// The actual size of the device now.
    pub fn current_capacity(&self) -> Sectors {
        let size = self.used.capacity();
        assert_eq!(self.bda.dev_size(), size);
        size
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

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }
}

impl BlockDev for StratBlockDev {
    fn devnode(&self) -> PathBuf {
        self.devnode.clone()
    }

    fn user_info(&self) -> Option<&str> {
        self.user_info.as_ref().map(|x| &**x)
    }

    fn hardware_info(&self) -> Option<&str> {
        self.hardware_info.as_ref().map(|x| &**x)
    }

    fn initialization_time(&self) -> DateTime<Utc> {
        // This cast will result in an incorrect, negative value starting in
        // the year 292,277,026,596. :-)
        Utc.timestamp(self.bda.initialization_time() as i64, 0)
    }

    fn total_size(&self) -> Sectors {
        let start = self.metadata_size();
        let size = self.current_capacity();
        assert!(start <= size);
        size - start
    }

    fn state(&self) -> BlockDevState {
        // TODO: Implement states for blockdevs
        BlockDevState::InUse
    }

    #[cfg(feature = "dbus_enabled")]
    fn set_dbus_path(&mut self, path: dbus::Path<'static>) -> () {
        self.user_data = Some(path)
    }

    #[cfg(feature = "dbus_enabled")]
    fn get_dbus_path(&self) -> &Option<dbus::Path<'static>> {
        &self.user_data
    }
}

impl Recordable<BlockDevSave> for StratBlockDev {
    fn record(&self) -> BlockDevSave {
        BlockDevSave {
            uuid: self.uuid(),
            user_info: self.user_info.clone(),
            hardware_info: self.hardware_info.clone(),
        }
    }
}
