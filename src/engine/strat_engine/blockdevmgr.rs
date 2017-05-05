// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a collection of block devices.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use devicemapper::{Sectors, Segment};
use time::Timespec;

use engine::{EngineResult, PoolUuid};
use super::metadata::MIN_MDA_SECTORS;
use engine::strat_engine::blockdev::{BlockDev, initialize};
use engine::strat_engine::device::resolve_devices;

use super::setup;

#[derive(Debug)]
pub struct BlockDevMgr {
    pub block_devs: HashMap<PathBuf, BlockDev>,
}

impl BlockDevMgr {
    pub fn new(block_devs: Vec<BlockDev>) -> BlockDevMgr {
        BlockDevMgr {
            block_devs: block_devs.into_iter().map(|bd| (bd.devnode.clone(), bd)).collect(),
        }
    }

    pub fn add(&mut self,
               pool_uuid: &PoolUuid,
               paths: &[&Path],
               force: bool)
               -> EngineResult<Vec<PathBuf>> {
        let devices = try!(resolve_devices(paths));
        let bds = try!(initialize(pool_uuid, devices, MIN_MDA_SECTORS, force));
        let bdev_paths = bds.iter().map(|p| p.devnode.clone()).collect();
        for bd in bds {
            self.block_devs.insert(bd.devnode.clone(), bd);
        }
        Ok(bdev_paths)
    }

    pub fn destroy_all(mut self) -> EngineResult<()> {
        for (_, bd) in self.block_devs.drain() {
            try!(bd.wipe_metadata());
        }
        Ok(())
    }

    // Unused space left on blockdevs
    pub fn avail_space(&self) -> Sectors {
        self.block_devs.values().map(|bd| bd.available()).sum()
    }

    /// If available space is less than size, return None, else return
    /// the segments allocated.
    pub fn alloc_space(&mut self, size: Sectors) -> Option<Vec<Segment>> {
        let mut needed: Sectors = size;
        let mut segs = Vec::new();

        if self.avail_space() < size {
            return None;
        }

        for mut bd in self.block_devs.values_mut() {
            if needed == Sectors(0) {
                break;
            }

            let (gotten, r_segs) = bd.request_space(needed);
            segs.extend(r_segs.iter()
                .map(|&(start, len)| Segment::new(bd.dev, start, len)));
            needed = needed - gotten;
        }

        assert_eq!(needed, Sectors(0));

        Some(segs)
    }

    pub fn devnodes(&self) -> Vec<PathBuf> {
        self.block_devs.keys().map(|p| p.clone()).collect()
    }

    /// Write the given data to all blockdevs marking with specified time.
    // TODO: Cap # of blockdevs written to, as described in SWDD
    pub fn save_state(&mut self, time: &Timespec, metadata: &[u8]) -> EngineResult<()> {
        // TODO: Do something better than panic when saving to blockdev fails.
        // Panic can occur for a the usual IO reasons, but also:
        // 1. If the timestamp is older than a previously written timestamp.
        // 2. If the variable length metadata is too large.
        for bd in self.block_devs.values_mut() {
            bd.save_state(time, metadata).unwrap();
        }
        Ok(())
    }

    /// Return the metadata from the first blockdev with up-to-date, readable
    /// metadata.
    pub fn load_state(&self) -> Option<Vec<u8>> {
        let blockdevs: Vec<&BlockDev> = self.block_devs.values().collect();
        setup::load_state(&blockdevs)
    }
}
