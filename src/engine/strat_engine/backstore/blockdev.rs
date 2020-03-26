// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::{DateTime, TimeZone, Utc};

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        engine::BlockDev,
        strat_engine::{
            backstore::{
                crypt::CryptHandle,
                metadata::{disown_device, BDAExtendedSize, MDADataSize, BDA},
                range_alloc::RangeAllocator,
                shared::BlockDevPath,
            },
            serde_structs::{BaseBlockDevSave, Recordable},
        },
        types::{DevUuid, MaybeDbusPath},
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug)]
pub struct StratBlockDev {
    dev: Device,
    devnode: BlockDevPath,
    bda: BDA,
    used: RangeAllocator,
    user_info: Option<String>,
    hardware_info: Option<String>,
    dbus_path: MaybeDbusPath,
    key_description: Option<String>,
}

impl StratBlockDev {
    /// Make a new BlockDev from the parameters.
    /// Allocate space for the Stratis metadata on the device.
    /// - dev: the device, identified by number
    /// - devnode: for encrypted devices, the logical and physical
    ///            paths; for unencrypted devices, the physical path
    /// - bda: the device's BDA
    /// - other_segments: segments claimed for non-Stratis metadata use
    /// - user_info: user settable identifying information
    /// - hardware_info: identifying information in the hardware
    /// - key_description: optional argument enabling encryption using
    ///                    the specified key in the kernel keyring
    /// Returns an error if it is impossible to allocate all segments on the
    /// device.
    /// NOTE: It is possible that the actual device size is greater than
    /// the recorded device size. In that case, the additional space available
    /// on the device is simply invisible to the blockdev. Consequently, it
    /// is invisible to the engine, and is not part of the total size value
    /// reported on the D-Bus.
    pub fn new(
        dev: Device,
        devnode: BlockDevPath,
        bda: BDA,
        upper_segments: &[(Sectors, Sectors)],
        user_info: Option<String>,
        hardware_info: Option<String>,
        key_description: Option<String>,
    ) -> StratisResult<StratBlockDev> {
        let mut segments = vec![(Sectors(0), bda.extended_size().sectors())];
        segments.extend(upper_segments);
        let allocator = RangeAllocator::new(bda.dev_size(), &segments)?;

        Ok(StratBlockDev {
            dev,
            devnode,
            bda,
            used: allocator,
            user_info,
            hardware_info,
            dbus_path: MaybeDbusPath(None),
            key_description,
        })
    }

    /// Returns the blockdev's Device
    pub fn device(&self) -> &Device {
        &self.dev
    }

    /// Return the path to the physical device on which data is stored either
    /// encrypted or unencrypted.
    pub fn physical_path(&self) -> &Path {
        self.devnode.physical_path()
    }

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
    pub fn disown(&self) -> StratisResult<()> {
        if !self.is_encrypted() {
            disown_device(
                &mut OpenOptions::new()
                    .write(true)
                    .open(self.devnode.metadata_path())?,
            )?;
            Ok(())
        } else if let Some(ref mut handle) = CryptHandle::setup(self.devnode.physical_path())? {
            handle.wipe()?;
            Ok(())
        } else {
            warn!(
                "Device {} was determined to not be a Stratis device; cannot \
                disown a device that does not belong to Stratis.",
                self.devnode.physical_path().display()
            );
            Err(StratisError::Error(
                "Device requested to be disowned did not belong to Stratis".to_string(),
            ))
        }
    }

    pub fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .open(self.devnode.metadata_path())?;
        self.bda.save_state(time, metadata, &mut f)
    }

    /// The device's UUID.
    pub fn uuid(&self) -> DevUuid {
        self.bda.dev_uuid()
    }

    /// Find some sector ranges that could be allocated. If more
    /// sectors are needed than are available, return partial results.
    /// If all sectors are desired, use available() method to get all.
    pub fn request_space(&mut self, size: Sectors) -> (Sectors, Vec<(Sectors, Sectors)>) {
        self.used.request(size)
    }

    // ALL SIZE METHODS (except size(), which is in BlockDev impl.)
    /// The number of Sectors on this device used by Stratis for metadata
    pub fn metadata_size(&self) -> BDAExtendedSize {
        self.bda.extended_size()
    }

    /// The number of Sectors on this device not allocated for any purpose.
    /// self.size() - self.metadata_size() >= self.available()
    pub fn available(&self) -> Sectors {
        self.used.available()
    }

    /// The maximum size of variable length metadata that can be accommodated.
    /// self.max_metadata_size() < self.metadata_size()
    pub fn max_metadata_size(&self) -> MDADataSize {
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
        self.devnode.metadata_path().to_owned()
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

    fn size(&self) -> Sectors {
        let size = self.used.size();
        assert_eq!(self.bda.dev_size(), size);
        size.sectors()
    }

    fn set_dbus_path(&mut self, path: MaybeDbusPath) {
        self.dbus_path = path
    }

    fn get_dbus_path(&self) -> &MaybeDbusPath {
        &self.dbus_path
    }

    fn is_encrypted(&self) -> bool {
        self.key_description.is_some()
    }
}

impl Recordable<BaseBlockDevSave> for StratBlockDev {
    fn record(&self) -> BaseBlockDevSave {
        BaseBlockDevSave {
            uuid: self.uuid(),
            user_info: self.user_info.clone(),
            hardware_info: self.hardware_info.clone(),
        }
    }
}
