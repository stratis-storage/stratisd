// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Code to handle a single block device.

use std::{
    cmp::Ordering,
    fmt,
    fs::{File, OpenOptions},
    io::Seek,
    path::Path,
};

use chrono::{DateTime, Utc};
use serde_json::Value;

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        crypt_metadata_size,
        engine::{BlockDev, DumpState},
        strat_engine::{
            backstore::{
                devices::BlockSizes,
                range_alloc::{PerDevSegments, RangeAllocator},
            },
            crypt::handle::v1::CryptHandle,
            device::blkdev_size,
            metadata::{
                disown_device, static_header, BDAExtendedSize, BlockdevSize, MDADataSize,
                MetadataLocation, StaticHeader, BDA,
            },
            serde_structs::{BaseBlockDevSave, Recordable},
            types::BDAResult,
        },
        types::{
            Compare, DevUuid, DevicePath, Diff, EncryptionInfo, KeyDescription, Name, PoolUuid,
            StateDiff, StratBlockDevDiff, StratSigblockVersion,
        },
    },
    stratis::{StratisError, StratisResult},
};

#[derive(Debug)]
pub enum UnderlyingDevice {
    Encrypted(CryptHandle),
    Unencrypted(DevicePath),
}

impl UnderlyingDevice {
    pub fn physical_path(&self) -> &Path {
        match self {
            UnderlyingDevice::Encrypted(handle) => handle.luks2_device_path(),
            UnderlyingDevice::Unencrypted(path) => path,
        }
    }

    pub fn metadata_path(&self) -> &Path {
        match self {
            UnderlyingDevice::Encrypted(handle) => handle.activated_device_path(),
            UnderlyingDevice::Unencrypted(path) => path,
        }
    }

    pub fn crypt_handle(&self) -> Option<&CryptHandle> {
        match self {
            UnderlyingDevice::Encrypted(handle) => Some(handle),
            UnderlyingDevice::Unencrypted(_) => None,
        }
    }

    pub fn crypt_handle_mut(&mut self) -> Option<&mut CryptHandle> {
        match self {
            UnderlyingDevice::Encrypted(handle) => Some(handle),
            UnderlyingDevice::Unencrypted(_) => None,
        }
    }
}

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

#[derive(Debug)]
pub struct StratBlockDev {
    dev: Device,
    pub(in super::super) bda: BDA,
    used: RangeAllocator,
    user_info: Option<String>,
    hardware_info: Option<String>,
    underlying_device: UnderlyingDevice,
    new_size: Option<Sectors>,
    blksizes: StratSectorSizes,
}

impl StratBlockDev {
    /// Make a new BlockDev from the parameters.
    /// Allocate space for the Stratis metadata on the device.
    /// - dev: the device, identified by number
    /// - devnode: for encrypted devices, the logical and physical
    ///            paths; for unencrypted devices, the physical path
    /// - bda: the device's BDA
    /// - other_segments: segments allocated outside Stratis metadata region
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
    ///
    /// Precondition: segments in other_segments do not overlap with Stratis
    /// metadata region.
    pub fn new(
        dev: Device,
        bda: BDA,
        other_segments: &[(Sectors, Sectors)],
        user_info: Option<String>,
        hardware_info: Option<String>,
        underlying_device: UnderlyingDevice,
    ) -> BDAResult<StratBlockDev> {
        let mut segments = vec![(Sectors(0), bda.extended_size().sectors())];
        segments.extend(other_segments);

        let allocator = match RangeAllocator::new(bda.dev_size(), &segments) {
            Ok(a) => a,
            Err(e) => return Err((e, bda)),
        };

        let base_blksizes = match OpenOptions::new()
            .read(true)
            .open(underlying_device.physical_path())
            .map_err(StratisError::from)
            .and_then(|f| BlockSizes::read(&f))
        {
            Ok(blksizes) => blksizes,
            Err(e) => return Err((e, bda)),
        };

        let blksizes = match underlying_device {
            UnderlyingDevice::Encrypted(_) => {
                let metadata_path = underlying_device.metadata_path();
                let crypt_blksizes = match OpenOptions::new()
                    .read(true)
                    .open(metadata_path)
                    .map_err(StratisError::from)
                    .and_then(|f| BlockSizes::read(&f))
                {
                    Ok(blksizes) => blksizes,
                    Err(e) => return Err((e, bda)),
                };

                StratSectorSizes {
                    base: base_blksizes,
                    crypt: Some(crypt_blksizes),
                }
            }
            UnderlyingDevice::Unencrypted(_) => StratSectorSizes {
                base: base_blksizes,
                crypt: None,
            },
        };

        Ok(StratBlockDev {
            dev,
            bda,
            used: allocator,
            user_info,
            hardware_info,
            underlying_device,
            new_size: None,
            blksizes,
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

    /// Returns the LUKS2 device's Device if encrypted
    pub fn luks_device(&self) -> Option<&Device> {
        self.underlying_device.crypt_handle().map(|ch| ch.device())
    }

    /// Returns the physical path of the block device structure.
    pub fn physical_path(&self) -> &Path {
        self.devnode()
    }

    /// Returns the path to the unencrypted metadata stored on the block device structure.
    /// On encrypted devices, this will point to a devicemapper device set up by libcryptsetup.
    /// On unencrypted devices, this will be the same as the physical device.
    pub fn metadata_path(&self) -> &Path {
        self.underlying_device.metadata_path()
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
    pub fn disown(&mut self) -> StratisResult<()> {
        if let Some(ref mut handle) = self.underlying_device.crypt_handle_mut() {
            handle.wipe()?;
        } else {
            disown_device(
                &mut OpenOptions::new()
                    .write(true)
                    .open(self.underlying_device.physical_path())?,
            )?;
        }
        Ok(())
    }

    pub fn save_state(&mut self, time: &DateTime<Utc>, metadata: &[u8]) -> StratisResult<()> {
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.underlying_device.metadata_path())?;
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

    pub fn load_state(&self) -> StratisResult<Option<(Vec<u8>, &DateTime<Utc>)>> {
        let mut f = OpenOptions::new()
            .read(true)
            .open(self.underlying_device.metadata_path())?;
        match (self.bda.load_state(&mut f)?, self.bda.last_update_time()) {
            (Some(state), Some(time)) => Ok(Some((state, time))),
            (None, None) => Ok(None),
            _ => Err(StratisError::Msg(
                "Stratis metadata written but unknown update time or vice-versa".into(),
            )),
        }
    }

    /// The pool's UUID.
    pub fn pool_uuid(&self) -> PoolUuid {
        self.bda.pool_uuid()
    }

    /// The device's UUID.
    pub fn uuid(&self) -> DevUuid {
        self.bda.dev_uuid()
    }

    /// Find some sector ranges that could be allocated. If more
    /// sectors are needed than are available, return partial results.
    pub fn alloc(&mut self, size: Sectors) -> PerDevSegments {
        self.used.alloc(size)
    }

    // ALL SIZE METHODS (except size(), which is in BlockDev impl.)
    /// The number of Sectors on this device used by Stratis for metadata
    pub fn metadata_size(&self) -> BDAExtendedSize {
        self.bda.extended_size()
    }

    /// The number of Sectors on this device not allocated for any purpose.
    /// self.total_allocated_size() - self.metadata_size() >= self.available()
    pub fn available(&self) -> Sectors {
        self.used.available()
    }

    /// The total size of the Stratis block device.
    pub fn total_size(&self) -> BlockdevSize {
        self.bda.dev_size()
    }

    /// The maximum size of variable length metadata that can be accommodated.
    /// self.max_metadata_size() < self.metadata_size()
    pub fn max_metadata_size(&self) -> MDADataSize {
        self.bda.max_data_size()
    }

    /// Whether or not the blockdev is in use by upper layers. It is if the
    /// sum of the blocks used exceeds the Stratis metadata size.
    pub fn in_use(&self) -> bool {
        self.used.used() > self.metadata_size().sectors()
    }

    /// Set the user info on this blockdev.
    /// The user_info may be None, which unsets user info.
    /// Returns true if the user info was changed, otherwise false.
    pub fn set_user_info(&mut self, user_info: Option<&str>) -> bool {
        set_blockdev_user_info!(self; user_info)
    }

    /// Get the physical path for a block device.
    pub fn devnode(&self) -> &Path {
        self.underlying_device.physical_path()
    }

    /// Get the encryption_info stored on the given encrypted blockdev.
    pub fn encryption_info(&self) -> Option<&EncryptionInfo> {
        self.underlying_device
            .crypt_handle()
            .map(|ch| ch.encryption_info())
    }

    /// Get the pool name for the given block device.
    ///
    /// Returns:
    /// * Some(Some(_)) if the pool is encrypted and the pool name is set in the
    /// metadata
    /// * Some(None) if the pool is encrypted and the pool name is not set in the
    /// metadata
    /// * None if the pool is not encrypted
    #[allow(clippy::option_option)]
    pub fn pool_name(&self) -> Option<Option<&Name>> {
        self.underlying_device
            .crypt_handle()
            .map(|ch| ch.pool_name())
    }

    /// Block size information
    pub fn blksizes(&self) -> StratSectorSizes {
        self.blksizes
    }

    /// Bind encrypted device using the given clevis configuration.
    pub fn bind_clevis(&mut self, pin: &str, clevis_info: &Value) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.clevis_bind(pin, clevis_info)
    }

    /// Unbind encrypted device using the given clevis configuration.
    pub fn unbind_clevis(&mut self) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.clevis_unbind()
    }

    /// Bind a block device to a passphrase represented by a key description
    /// in the kernel keyring.
    pub fn bind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.bind_keyring(key_desc)
    }

    /// Unbind a block device from a passphrase represented by a key description
    /// in the kernel keyring.
    pub fn unbind_keyring(&mut self) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.unbind_keyring()
    }

    /// Change the passphrase for a block device to a passphrase represented by a
    /// key description in the kernel keyring.
    pub fn rebind_keyring(&mut self, key_desc: &KeyDescription) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.rebind_keyring(key_desc)
    }

    /// Regenerate the Clevis bindings for a block device.
    pub fn rebind_clevis(&mut self) -> StratisResult<()> {
        let crypt_handle = self.underlying_device.crypt_handle_mut().ok_or_else(|| {
            StratisError::Msg("This device does not appear to be encrypted".to_string())
        })?;
        crypt_handle.rebind_clevis()
    }

    /// Calculate the new size of the block device specified by physical_path.
    ///
    /// Returns:
    /// * `None` if the size hasn't changed or is equal to the current size recorded
    /// in the metadata.
    /// * Otherwise, `Some(_)`
    pub fn calc_new_size(&self) -> StratisResult<Option<Sectors>> {
        let s = Self::scan_blkdev_size(
            self.physical_path(),
            self.underlying_device.crypt_handle().is_some(),
        )?;
        if Some(s) == self.new_size
            || (self.new_size.is_none() && s == self.bda.dev_size().sectors())
        {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    /// Scan the block device specified by physical_path for its size.
    pub fn scan_blkdev_size(physical_path: &Path, is_encrypted: bool) -> StratisResult<Sectors> {
        Ok(blkdev_size(&File::open(physical_path)?)?.sectors()
            - if is_encrypted {
                crypt_metadata_size().sectors()
            } else {
                Sectors(0)
            })
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
    /// This method does not need to block IO to the extended crypt device prior
    /// to rollback because of per-pool locking. Growing the device will acquire
    /// an exclusive lock on the pool and therefore the thin pool cannot be
    /// extended to use the larger or unencrypted block device size until the
    /// transaction has been completed successfully.
    pub fn grow(&mut self) -> StratisResult<bool> {
        /// Precondition: size > h.blkdev_size
        fn needs_rollback(bd: &mut StratBlockDev, size: BlockdevSize) -> StratisResult<()> {
            let mut f = OpenOptions::new()
                .write(true)
                .read(true)
                .open(bd.metadata_path())?;
            let mut h = static_header(&mut f)?.ok_or_else(|| {
                StratisError::Msg(format!(
                    "No static header found on device {}",
                    bd.metadata_path().display()
                ))
            })?;

            h.blkdev_size = size;
            let h = StaticHeader::write_header(&mut f, h, MetadataLocation::Both)?;

            bd.bda.header = h;
            bd.used.increase_size(size.sectors());

            Ok(())
        }

        fn rollback(
            causal_error: StratisError,
            info: Option<(&CryptHandle, BlockdevSize)>,
        ) -> StratisError {
            if let Some((h, cs)) = info {
                if let Err(e) = h.resize(Some(cs.sectors())) {
                    StratisError::NoActionRollbackError { causal_error: Box::new(causal_error), rollback_error: Box::new(StratisError::Chained("Failed to rollback crypt device growth; no changes were made to the Stratis metadata so no IO will be written to the newly extended portions of the crypt device; future growth operations on the same device are safe".to_string(), Box::new(e))) }
                } else {
                    causal_error
                }
            } else {
                causal_error
            }
        }

        let size = BlockdevSize::new(Self::scan_blkdev_size(
            self.physical_path(),
            self.underlying_device.crypt_handle().is_some(),
        )?);
        let metadata_size = self.bda.dev_size();
        match size.cmp(&metadata_size) {
            Ordering::Less => Err(StratisError::Msg(
                "The underlying device appears to have shrunk; you may experience data loss"
                    .to_string(),
            )),
            Ordering::Equal => Ok(false),
            Ordering::Greater => {
                let info = if let Some(h) = self.underlying_device.crypt_handle() {
                    let orig_size = BlockdevSize::new(
                        blkdev_size(&File::open(self.metadata_path())?)?.sectors(),
                    );
                    h.resize(None)?;
                    Some(orig_size)
                } else {
                    None
                };

                if let Err(e) = needs_rollback(self, size) {
                    return Err(rollback(
                        e,
                        info.and_then(|os| self.underlying_device.crypt_handle().map(|h| (h, os))),
                    ));
                }

                Ok(true)
            }
        }
    }

    /// Rename pool in metadata if it is encrypted.
    pub fn rename_pool(&mut self, pool_name: Name) -> StratisResult<()> {
        match self.underlying_device.crypt_handle_mut() {
            Some(handle) => handle.rename_pool_in_metadata(pool_name),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    pub fn invariant(&self) {
        assert!(self.total_size() == self.used.size());
    }

    /// If a pool is encrypted, tear down the cryptsetup devicemapper devices on the
    /// physical device.
    pub fn teardown(&mut self) -> StratisResult<()> {
        if let Some(ch) = self.underlying_device.crypt_handle() {
            debug!(
                "Deactivating unlocked encrypted device with UUID {}",
                self.bda.dev_uuid()
            );
            ch.deactivate()
        } else {
            Ok(())
        }
    }

    /// Get metadata version from static header
    pub fn metadata_version(&self) -> StratSigblockVersion {
        self.bda.sigblock_version()
    }
}

impl<'a> Into<Value> for &'a StratBlockDev {
    fn into(self) -> Value {
        let mut json = json!({
            "path": self.underlying_device.physical_path(),
            "uuid": self.bda.dev_uuid().to_string(),
        });
        let map = json.as_object_mut().expect("just created above");
        if let Some(encryption_info) = self
            .underlying_device
            .crypt_handle()
            .map(|ch| ch.encryption_info())
        {
            if let Value::Object(enc_map) = <&EncryptionInfo as Into<Value>>::into(encryption_info)
            {
                map.extend(enc_map);
            } else {
                unreachable!("EncryptionInfo conversion returns a JSON object");
            };
        }
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
        self.metadata_path()
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

    fn is_encrypted(&self) -> bool {
        self.encryption_info().is_some()
    }

    fn new_size(&self) -> Option<Sectors> {
        self.new_size
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

impl<'a> DumpState<'a> for StratBlockDev {
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
