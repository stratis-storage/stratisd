// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::{
    cmp::Ordering,
    fs::{File, OpenOptions},
    io::Seek,
    path::Path,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{Bytes, Device, Sectors};

use crate::{
    engine::{
        engine::{BlockDev, DumpState},
        strat_engine::{
            backstore::{
                blockdev::{InternalBlockDev, StratSectorSizes},
                devices::BlockSizes,
                range_alloc::{PerDevSegments, RangeAllocator},
            },
            device::blkdev_size,
            metadata::{
                disown_device, static_header, BlockdevSize, MDADataSize, MetadataLocation,
                StaticHeader, BDA,
            },
            serde_structs::{BaseBlockDevSave, Recordable},
        },
        types::{
            Compare, DevUuid, DevicePath, Diff, PoolUuid, StateDiff, StratBlockDevDiff,
            StratSigblockVersion, ValidatedIntegritySpec,
        },
    },
    stratis::{StratisError, StratisResult},
};

/// Return the amount of space required for integrity for a device of the given size.
///
/// This default is a slight overestimation for the sake of simplicity. Because it uses the whole disk
/// size, once the integrity metadata size is calculated, the remaining data size is now smaller
/// than the metadata region could support for integrity.
/// The result is divisible by 8 sectors.
pub fn integrity_meta_space(
    total_space: Sectors,
    integrity_spec: ValidatedIntegritySpec,
) -> Sectors {
    (if integrity_spec.allocate_superblock {
        Bytes(4096)
    } else {
        Bytes(0)
    })
    .sectors()
        + integrity_spec.journal_size
        + Bytes::from(
            (*((total_space.bytes() / integrity_spec.block_size)
                * integrity_spec.tag_spec.as_bytes_ceil())
                + 4095)
                & !4095,
        )
        .sectors()
}

#[derive(Debug)]
pub struct StratBlockDev {
    dev: Device,
    bda: BDA,
    used: RangeAllocator,
    user_info: Option<String>,
    hardware_info: Option<String>,
    devnode: DevicePath,
    new_size: Option<Sectors>,
    blksizes: StratSectorSizes,
    integrity_meta_allocs: Vec<(Sectors, Sectors)>,
}

impl StratBlockDev {
    /// Make a new BlockDev from the parameters.
    /// Allocate space for the Stratis metadata on the device.
    /// - dev: the device, identified by number
    /// - devnode: for encrypted devices, the logical and physical
    ///   paths; for unencrypted devices, the physical path
    /// - bda: the device's BDA
    /// - other_segments: segments allocated outside Stratis metadata region
    /// - user_info: user settable identifying information
    /// - hardware_info: identifying information in the hardware
    /// - key_description: optional argument enabling encryption using
    ///   the specified key in the kernel keyring
    ///
    /// Returns an error if it is impossible to allocate all segments on the
    /// device.
    /// NOTE: It is possible that the actual device size is greater than
    /// the recorded device size. In that case, the additional space available
    /// on the device is simply invisible to the blockdev. Consequently, it
    /// is invisible to the engine, and is not part of the total size value
    /// reported on the D-Bus.
    ///
    /// Precondition: segments in other_segments do not overlap with Stratis
    /// metadata region.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dev: Device,
        bda: BDA,
        other_segments: &[(Sectors, Sectors)],
        integrity_meta_allocs: &[(Sectors, Sectors)],
        user_info: Option<String>,
        hardware_info: Option<String>,
        devnode: DevicePath,
    ) -> StratisResult<StratBlockDev> {
        let mut segments = vec![(Sectors(0), bda.extended_size().sectors())];
        segments.extend(other_segments);
        segments.extend(integrity_meta_allocs);

        let allocator = RangeAllocator::new(bda.dev_size(), &segments)?;

        let base_blksizes = OpenOptions::new()
            .read(true)
            .open(&*devnode)
            .map_err(StratisError::from)
            .and_then(|f| BlockSizes::read(&f))?;

        let blksizes = StratSectorSizes {
            base: base_blksizes,
            crypt: None,
        };

        Ok(StratBlockDev {
            dev,
            bda,
            used: allocator,
            user_info,
            hardware_info,
            devnode,
            new_size: None,
            blksizes,
            integrity_meta_allocs: integrity_meta_allocs.to_owned(),
        })
    }

    /// Returns the blockdev's Device. For unencrypted devices, this is the physical,
    /// unencrypted device. For encrypted devices, this is the logical, unlocked
    /// device on top of LUKS2.
    ///
    /// Practically, this is the device number that should be used when constructing
    /// the cap device.
    pub fn device(&self) -> &Device {
        &self.dev
    }

    pub fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()> {
        let mut f = OpenOptions::new().write(true).open(self.devnode())?;
        self.bda.save_state(time, metadata, &mut f)
    }

    /// The pool's UUID.
    pub fn pool_uuid(&self) -> PoolUuid {
        self.bda.pool_uuid()
    }

    /// The device's UUID.
    pub fn uuid(&self) -> DevUuid {
        self.bda.dev_uuid()
    }

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }

    /// Get the physical path for a block device.
    pub fn devnode(&self) -> &Path {
        &self.devnode
    }

    /// Scan the block device specified by physical_path for its size.
    pub fn scan_blkdev_size(physical_path: &Path) -> StratisResult<Sectors> {
        Ok(blkdev_size(&File::open(physical_path)?)?.sectors())
    }

    /// Allocate room for integrity metadata from the back of the device.
    pub fn alloc_int_meta_back(&mut self, size: Sectors) {
        let segs = self.used.alloc_back(size);
        for (start, len) in segs.iter() {
            self.integrity_meta_allocs.push((*start, *len));
        }
    }

    /// Set the newly detected size of a block device.
    pub fn set_new_size(&mut self, new_size: Sectors) {
        match self.bda.dev_size().cmp(&BlockdevSize::new(new_size)) {
            Ordering::Greater => {
                warn!(
                    "The given device with path: {}, UUID; {} appears to have shrunk; you may experience data loss",
                    self.devnode().display(),
                    self.bda.dev_uuid(),
                );
                self.new_size = Some(new_size);
            }
            Ordering::Less => {
                self.new_size = Some(new_size);
            }
            Ordering::Equal => {
                self.new_size = None;
            }
        }
    }

    /// Grow the block device if the underlying physical device has grown in size.
    /// Return an error and leave the size as is if the device has shrunk.
    /// Do nothing if the device is the same size as recorded in the metadata.
    ///
    /// This will also extend integrity metadata reservations according to the new
    /// size of the device.
    pub fn grow(&mut self, integrity_spec: ValidatedIntegritySpec) -> StratisResult<bool> {
        let size = BlockdevSize::new(Self::scan_blkdev_size(self.devnode())?);
        let metadata_size = self.bda.dev_size();
        match size.cmp(&metadata_size) {
            Ordering::Less => Err(StratisError::Msg(
                "The underlying device appears to have shrunk; you may experience data loss"
                    .to_string(),
            )),
            Ordering::Equal => Ok(false),
            Ordering::Greater => {
                let mut f = OpenOptions::new()
                    .write(true)
                    .read(true)
                    .open(self.devnode())?;
                let mut h = static_header(&mut f)?.ok_or_else(|| {
                    StratisError::Msg(format!(
                        "No static header found on device {}",
                        self.devnode().display()
                    ))
                })?;

                h.blkdev_size = size;
                let h = StaticHeader::write_header(&mut f, h, MetadataLocation::Both)?;

                self.bda.header = h;
                self.used.increase_size(size.sectors());

                let integrity_grow = integrity_meta_space(size.sectors(), integrity_spec)
                    - self
                        .integrity_meta_allocs
                        .iter()
                        .map(|(_, len)| *len)
                        .sum::<Sectors>();
                self.alloc_int_meta_back(integrity_grow);

                Ok(true)
            }
        }
    }

    #[cfg(test)]
    pub fn invariant(&self) {
        assert!(self.total_size() == self.used.size());
    }
}

impl InternalBlockDev for StratBlockDev {
    fn bda(&self) -> &BDA {
        &self.bda
    }

    fn uuid(&self) -> DevUuid {
        self.bda.dev_uuid()
    }

    fn device(&self) -> &Device {
        &self.dev
    }

    fn physical_path(&self) -> &Path {
        &self.devnode
    }

    fn blksizes(&self) -> StratSectorSizes {
        self.blksizes
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        self.bda.sigblock_version()
    }

    fn total_size(&self) -> BlockdevSize {
        self.bda.dev_size()
    }

    fn available(&self) -> Sectors {
        self.used.available()
    }

    fn metadata_size(&self) -> Sectors {
        self.bda.extended_size().sectors()
            + self.integrity_meta_allocs.iter().map(|(_, len)| *len).sum()
    }

    fn max_stratis_metadata_size(&self) -> MDADataSize {
        self.bda.max_data_size()
    }

    fn in_use(&self) -> bool {
        self.used.used() > self.metadata_size()
    }

    fn alloc(&mut self, size: Sectors) -> PerDevSegments {
        self.used.alloc_front(size)
    }

    fn calc_new_size(&self) -> StratisResult<Option<Sectors>> {
        let s = Self::scan_blkdev_size(self.devnode())?;
        if Some(s) == self.new_size
            || (self.new_size.is_none() && s == self.bda.dev_size().sectors())
        {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    fn load_state(&self) -> StratisResult<Option<(Vec<u8>, &DateTime<Utc>)>> {
        let mut f = OpenOptions::new().read(true).open(&*self.devnode)?;
        match (self.bda.load_state(&mut f)?, self.bda.last_update_time()) {
            (Some(state), Some(time)) => Ok(Some((state, time))),
            (None, None) => Ok(None),
            _ => Err(StratisError::Msg(
                "Stratis metadata written but unknown update time or vice-versa".into(),
            )),
        }
    }

    fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()> {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&*self.devnode)?;
        self.bda.save_state(time, metadata, &mut f)?;

        f.rewind()?;
        let header = static_header(&mut f)?.ok_or_else(|| {
            StratisError::Msg("Stratis device has no signature buffer".to_string())
        })?;
        let bda = BDA::load(header, &mut f)?
            .ok_or_else(|| StratisError::Msg("Stratis device has no BDA".to_string()))?;
        self.bda = bda;
        Ok(())
    }

    fn teardown(&mut self) -> StratisResult<()> {
        Ok(())
    }

    fn disown(&mut self) -> StratisResult<()> {
        disown_device(&mut OpenOptions::new().write(true).open(self.devnode())?)?;
        Ok(())
    }
}

impl Into<Value> for &StratBlockDev {
    fn into(self) -> Value {
        let mut json = json!({
            "path": self.devnode(),
            "uuid": self.bda.dev_uuid().to_string(),
        });
        let map = json.as_object_mut().expect("just created above");
        map.insert("size".to_string(), Value::from(self.size().to_string()));
        if let Some(new_size) = self.new_size {
            map.insert("new_size".to_string(), Value::from(new_size.to_string()));
        }
        map.insert(
            "blksizes".to_string(),
            Value::from(self.blksizes.to_string()),
        );
        map.insert("in_use".to_string(), Value::from(self.in_use()));
        json
    }
}

impl BlockDev for StratBlockDev {
    fn devnode(&self) -> &Path {
        self.devnode()
    }

    fn metadata_path(&self) -> &Path {
        self.devnode()
    }

    fn user_info(&self) -> Option<&str> {
        self.user_info.as_deref()
    }

    fn hardware_info(&self) -> Option<&str> {
        self.hardware_info.as_deref()
    }

    fn initialization_time(&self) -> DateTime<Utc> {
        self.bda.initialization_time()
    }

    fn size(&self) -> Sectors {
        self.total_size().sectors()
    }

    fn new_size(&self) -> Option<Sectors> {
        self.new_size
    }

    fn metadata_version(&self) -> StratSigblockVersion {
        self.bda.sigblock_version()
    }
}

impl Recordable<BaseBlockDevSave> for StratBlockDev {
    fn record(&self) -> BaseBlockDevSave {
        BaseBlockDevSave {
            uuid: self.uuid(),
            user_info: self.user_info.clone(),
            hardware_info: self.hardware_info.clone(),
            integrity_meta_allocs: self.integrity_meta_allocs.clone(),
        }
    }
}

pub struct StratBlockDevState {
    new_size: Option<Sectors>,
}

impl StateDiff for StratBlockDevState {
    type Diff = StratBlockDevDiff;

    fn diff(&self, new_state: &Self) -> Self::Diff {
        StratBlockDevDiff {
            size: self.new_size.compare(&new_state.new_size),
        }
    }

    fn unchanged(&self) -> Self::Diff {
        StratBlockDevDiff {
            size: Diff::Unchanged(self.new_size),
        }
    }
}

impl DumpState<'_> for StratBlockDev {
    type State = StratBlockDevState;
    type DumpInput = Sectors;

    fn cached(&self) -> Self::State {
        StratBlockDevState {
            new_size: self.new_size,
        }
    }

    fn dump(&mut self, input: Self::DumpInput) -> Self::State {
        self.set_new_size(input);
        StratBlockDevState {
            new_size: self.new_size,
        }
    }
}
