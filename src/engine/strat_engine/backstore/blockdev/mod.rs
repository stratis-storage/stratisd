// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fmt, path::Path};

use chrono::{DateTime, Utc};

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{devices::BlockSizes, range_alloc::PerDevSegments},
            metadata::{BlockdevSize, MDADataSize, BDA},
        },
        types::{DevUuid, StratSigblockVersion},
    },
    stratis::StratisResult,
};

pub mod v1;
pub mod v2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StratSectorSizes {
    pub base: BlockSizes,
    pub crypt: Option<BlockSizes>,
}

impl fmt::Display for StratSectorSizes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "base: {}, crypt: {}",
            self.base,
            &self
                .crypt
                .map(|sz| sz.to_string())
                .unwrap_or("None".to_string())
        )
    }
}

pub trait InternalBlockDev {
    /// The BDA.
    fn bda(&self) -> &BDA;

    /// The device's UUID.
    fn uuid(&self) -> DevUuid;

    /// Returns the blockdev's Device. For unencrypted devices, this is the physical,
    /// unencrypted device. For encrypted devices, this is the logical, unlocked
    /// device on top of LUKS2.
    ///
    /// Practically, this is the device number that should be used when constructing
    /// the cap device.
    fn device(&self) -> &Device;

    /// Returns the physical path of the block device structure.
    fn physical_path(&self) -> &Path;

    /// Block size information
    fn blksizes(&self) -> StratSectorSizes;

    /// Return sigblock metadata version for this block device.
    fn metadata_version(&self) -> StratSigblockVersion;

    /// The total size of the Stratis block device.
    fn total_size(&self) -> BlockdevSize;

    /// The number of Sectors on this device not allocated for any purpose.
    /// self.total_allocated_size() - self.metadata_size() >= self.available()
    fn available(&self) -> Sectors;

    // ALL SIZE METHODS (except size(), which is in BlockDev impl.)
    /// The number of Sectors on this device used by Stratis and potentially other devicemapper
    /// layers for metadata
    fn metadata_size(&self) -> Sectors;

    /// The maximum size of variable length Stratis metadata that can be accommodated.
    /// self.max_metadata_size() < self.metadata_size()
    fn max_stratis_metadata_size(&self) -> MDADataSize;

    /// Whether or not the blockdev is in use by upper layers. It is if the
    /// sum of the blocks used exceeds the Stratis metadata size.
    fn in_use(&self) -> bool;

    /// Find some sector ranges that could be allocated. If more
    /// sectors are needed than are available, return partial results.
    fn alloc(&mut self, size: Sectors) -> PerDevSegments;

    /// Calculate the new size of the block device specified by physical_path.
    ///
    /// Returns:
    /// * `None` if the size hasn't changed or is equal to the current size recorded
    ///   in the metadata.
    /// * Otherwise, `Some(_)`
    fn calc_new_size(&self) -> StratisResult<Option<Sectors>>;

    /// Load the pool-level metadata for the given block device.
    fn load_state(&self) -> StratisResult<Option<(Vec<u8>, &DateTime<Utc>)>>;

    /// Save the current metadata state to block device.
    fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()>;

    /// If a pool is encrypted, tear down the cryptsetup devicemapper devices on the
    /// physical device.
    fn teardown(&mut self) -> StratisResult<()>;

    /// Remove information that identifies this device as belonging to Stratis
    ///
    /// If self.is_encrypted() is true, destroy all keyslots and wipe the LUKS2 header.
    /// This will render all Stratis and LUKS2 metadata unreadable and unrecoverable
    /// from the given device.
    ///
    /// If self.is_encrypted() is false, wipe the Stratis metadata on the device.
    /// This will make the Stratis data and metadata invisible to all standard blkid
    /// and stratisd operations.
    ///
    /// Precondition: if self.is_encrypted() == true, the data on
    ///               self.devnode.physical_path() has been encrypted with
    ///               aes-xts-plain64 encryption.
    fn disown(&mut self) -> StratisResult<()>;
}
