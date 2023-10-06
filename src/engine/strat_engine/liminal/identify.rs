// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Discover or identify devices that may belong to Stratis.
//!
//! This module contains methods for finding all block devices that may belong
//! to Stratis, generally run when Stratis starts up, and other methods that
//! may be used to classify a single unknown block device.
//!
//! The methods rely to a greater or lesser extent on libudev.
//!
//! They have the following invocation hierarchy:
//! find_all*
//!  |
//! find_all_*_devices
//!  |
//! identify_*_device
//!  |
//! process_*_device
//!
//! The primary purpose of the find* methods is to construct a udev
//! enumeration and to properly process each of the udev database entries
//! found.
//!
//! The primary purpose of the identify_* methods is to use udev to identify
//! a single device and take the appropriate action based on that
//! identification.
//!
//! The primary purpose of the process_* methods is to gather up Stratis
//! device identifiers.
//!
//! Each method is expected to be invoked in a particular situation which
//! is guaranteed by the method that invokes it. The methods are not,
//! in general, general purpose methods that can be used in any situation.
//!
//! find_all is public because it is the method that is invoked by the
//! engine on startup. identify_block_device is public because it
//! is suitable for identifying a block device associated with a uevent.

use std::{
    collections::HashMap,
    fmt,
    fs::OpenOptions,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use serde_json::Value;

use devicemapper::Device;

use crate::engine::{
    strat_engine::{
        backstore::{CryptHandle, StratBlockDev},
        metadata::{static_header, StratisIdentifiers, BDA},
        udev::{
            block_enumerator, decide_ownership, UdevOwnership, CRYPTO_FS_TYPE, FS_TYPE_KEY,
            STRATIS_FS_TYPE,
        },
    },
    types::{EncryptionInfo, Name, PoolUuid, UdevEngineDevice, UdevEngineEvent},
};

/// Information related to device number and path for either a Stratis device
/// or a Stratis LUKS2 device.
#[derive(Debug, Hash, PartialEq, Eq)]
pub struct StratisDevInfo {
    pub device_number: Device,
    pub devnode: PathBuf,
}

impl<'a> Into<Value> for &'a StratisDevInfo {
    // Precondition: (&StratisIdentifiers).into() pattern matches
    // Value::Object()
    fn into(self) -> Value {
        json!({
            "major": Value::from(self.device_number.major),
            "minor": Value::from(self.device_number.minor),
            "devnode": Value::from(self.devnode.display().to_string())
        })
    }
}

impl fmt::Display for StratisDevInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "device number: \"{}\", devnode: \"{}\"",
            self.device_number,
            self.devnode.display()
        )
    }
}

/// A miscellaneous group of identifiers found when identifying a LUKS
/// device which belongs to Stratis.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct LuksInfo {
    pub dev_info: StratisDevInfo,
    pub identifiers: StratisIdentifiers,
    /// Encryption information
    pub encryption_info: EncryptionInfo,
    /// Name of the pool stored in LUKS2 Stratis token
    pub pool_name: Option<Name>,
}

impl fmt::Display for LuksInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, {}, {}",
            self.dev_info, self.identifiers, self.encryption_info,
        )
    }
}

/// A miscellaneous group of identifiers found when identifying a Stratis
/// device.
#[derive(Debug)]
pub struct StratisInfo {
    pub bda: BDA,
    pub dev_info: StratisDevInfo,
}

impl PartialEq for StratisInfo {
    fn eq(&self, rhs: &Self) -> bool {
        self.bda.identifiers() == rhs.bda.identifiers() && self.dev_info == rhs.dev_info
    }
}

impl Eq for StratisInfo {}

impl Hash for StratisInfo {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        self.bda.identifiers().hash(hasher);
        self.dev_info.hash(hasher);
    }
}

impl fmt::Display for StratisInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.bda.identifiers(), self.dev_info,)
    }
}

/// An enum type to distinguish between LUKS devices belong to Stratis and
/// Stratis devices.
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum DeviceInfo {
    Luks(LuksInfo),
    Stratis(StratisInfo),
}

impl DeviceInfo {
    pub fn stratis_identifiers(&self) -> StratisIdentifiers {
        match self {
            DeviceInfo::Luks(info) => info.identifiers,
            DeviceInfo::Stratis(info) => info.bda.identifiers(),
        }
    }

    pub fn encryption_info(&self) -> Option<&EncryptionInfo> {
        match self {
            DeviceInfo::Luks(info) => Some(&info.encryption_info),
            DeviceInfo::Stratis(_) => None,
        }
    }
}

impl From<StratBlockDev> for Vec<DeviceInfo> {
    fn from(bd: StratBlockDev) -> Self {
        let mut device_infos = Vec::new();
        match (bd.encryption_info(), bd.pool_name(), bd.luks_device()) {
            (Some(ei), Some(pname), Some(dev)) => {
                if bd.physical_path().exists() {
                    device_infos.push(DeviceInfo::Luks(LuksInfo {
                        encryption_info: ei.clone(),
                        dev_info: StratisDevInfo {
                            device_number: *dev,
                            devnode: bd.physical_path().to_owned(),
                        },
                        identifiers: StratisIdentifiers {
                            pool_uuid: bd.pool_uuid(),
                            device_uuid: bd.uuid(),
                        },
                        pool_name: pname.cloned(),
                    }));
                    if bd.metadata_path().exists() {
                        device_infos.push(DeviceInfo::Stratis(StratisInfo {
                            dev_info: StratisDevInfo {
                                device_number: *bd.device(),
                                devnode: bd.metadata_path().to_owned(),
                            },
                            bda: bd.bda,
                        }));
                    }
                }
            }
            (None, None, None) => device_infos.push(DeviceInfo::Stratis(StratisInfo {
                dev_info: StratisDevInfo {
                    device_number: *bd.device(),
                    devnode: bd.physical_path().to_owned(),
                },
                bda: bd.bda,
            })),
            (_, _, _) => unreachable!("If bd.is_encrypted(), all are Some(_)"),
        }
        device_infos
    }
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceInfo::Luks(info) => write!(f, "LUKS device description: {info}"),
            DeviceInfo::Stratis(info) => write!(f, "Stratis device description: {info}"),
        }
    }
}

// A wrapper for obtaining the device number as a devicemapper Device
// which interprets absence of the value as an error, which it is in this
// context.
fn device_to_devno_wrapper(device: &UdevEngineDevice) -> Result<Device, String> {
    device
        .devnum()
        .ok_or_else(|| "udev entry did not contain a device number".into())
        .map(Device::from)
}

// A wrapper around the metadata module's process for reading the BDA
// which also handles failure to open a device for reading.
// Returns an error if the device could not be opened for reading.
// Returns Ok(Err(...)) if there was an error while reading the
// BDA from the device.
// Returns Ok(Ok(None)) if the BDA did not appear to be on the device.
pub fn bda_wrapper(devnode: &Path) -> Result<Result<Option<BDA>, String>, String> {
    OpenOptions::new()
        .read(true)
        .open(devnode)
        .as_mut()
        .map_err(|err| {
            format!(
                "device {} could not be opened for reading: {}",
                devnode.display(),
                err
            )
        })
        .map(|f| {
            let header =
                static_header(f).map_err(|_| "failed to read static header".to_string())?;
            match header {
                Some(h) => Ok(BDA::load(h, f)
                    .map_err(|_| "failed to read MDA or MDA was invalid".to_string())?),
                None => Ok(None),
            }
        })
}

/// Process a device which udev information indicates is a LUKS device.
fn process_luks_device(dev: &UdevEngineDevice) -> Option<LuksInfo> {
    match dev.devnode() {
        Some(devnode) => match CryptHandle::load_metadata(devnode) {
            Ok(None) => None,
            Err(err) => {
                warn!(
                            "udev identified device {} as a LUKS device, but could not read LUKS header from the device, disregarding the device: {}",
                            devnode.display(),
                            err,
                            );
                None
            }
            Ok(Some(metadata)) => Some(LuksInfo {
                dev_info: StratisDevInfo {
                    device_number: metadata.device,
                    devnode: metadata.physical_path.to_path_buf(),
                },
                identifiers: metadata.identifiers,
                encryption_info: metadata.encryption_info,
                pool_name: metadata.pool_name,
            }),
        },
        None => {
            warn!("udev identified a device as a LUKS2 device, but the udev entry for the device had no device node, disregarding device");
            None
        }
    }
}

/// Process a device which udev information indicates is a Stratis device.
fn process_stratis_device(dev: &UdevEngineDevice) -> Option<StratisInfo> {
    match dev.devnode() {
        Some(devnode) => {
            match (device_to_devno_wrapper(dev), bda_wrapper(devnode)) {
                (Err(err), _) | (_, Err(err) | Ok(Err(err))) => {
                    warn!("udev identified device {} as a Stratis device but {}, disregarding the device",
                          devnode.display(),
                          err);
                    None
                }
                (_, Ok(Ok(None))) => {
                    warn!("udev identified device {} as a Stratis device but there appeared to be no Stratis metadata on the device, disregarding the device",
                          devnode.display());
                    None
                }
                (Ok(device_number), Ok(Ok(Some(bda)))) => Some(StratisInfo {
                    bda,
                    dev_info: StratisDevInfo {
                        device_number,
                        devnode: devnode.to_path_buf(),
                    },
                }),
            }
        }
        None => {
            warn!("udev identified a device as a Stratis device, but the udev entry for the device had no device node, disregarding device");
            None
        }
    }
}

// Find all devices identified by udev and cryptsetup as LUKS devices
// belonging to Stratis.
fn find_all_luks_devices() -> libudev::Result<HashMap<PoolUuid, Vec<LuksInfo>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;
    enumerator.match_property(FS_TYPE_KEY, CRYPTO_FS_TYPE)?;

    let pool_map = enumerator
        .scan_devices()?
        .filter_map(|dev| identify_luks_device(&UdevEngineDevice::from(&dev)))
        .fold(HashMap::<PoolUuid, Vec<_>>::new(), |mut acc, info| {
            acc.entry(info.identifiers.pool_uuid)
                .or_default()
                .push(info);
            acc
        });
    Ok(pool_map)
}

// Find all devices identified by udev as Stratis devices.
fn find_all_stratis_devices() -> libudev::Result<HashMap<PoolUuid, Vec<StratisInfo>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = block_enumerator(&context)?;
    enumerator.match_property(FS_TYPE_KEY, STRATIS_FS_TYPE)?;

    let pool_map = enumerator
        .scan_devices()?
        .filter_map(|dev| identify_stratis_device(&UdevEngineDevice::from(&dev)))
        .fold(HashMap::<PoolUuid, Vec<_>>::new(), |mut acc, info| {
            acc.entry(info.bda.identifiers().pool_uuid)
                .or_default()
                .push(info);
            acc
        });
    Ok(pool_map)
}

// Identify a device that udev enumeration has already picked up as a LUKS
// device. Return None if the device does not, after all, appear to be a LUKS
// device belonging to Stratis. Log anything unusual at an appropriate level.
fn identify_luks_device(dev: &UdevEngineDevice) -> Option<LuksInfo> {
    let initialized = dev.is_initialized();
    if !initialized {
        warn!("Found a udev entry for a device identified as a Stratis device, but udev also identified it as uninitialized, disregarding the device");
        return None;
    };

    match decide_ownership(dev) {
        Err(err) => {
            warn!("Could not determine ownership of a block device identified as a LUKS device by udev, disregarding the device: {}",
                  err);
            None
        }
        Ok(ownership) => match ownership {
            UdevOwnership::Luks => process_luks_device(dev),
            UdevOwnership::MultipathMember => None,
            _ => {
                warn!("udev enumeration identified this device as a LUKS block device but on further examination udev identifies it as a {}",
                      ownership);
                None
            }
        },
    }
    .map(|info| {
        info!("LUKS block device belonging to Stratis with {} discovered during initial search",
              info,
        );
        info
    })
}

// Identify a device that udev enumeration has already picked up as a Stratis
// device. Return None if the device does not, after all, appear to be a Stratis
// device. Log anything unusual at an appropriate level.
fn identify_stratis_device(dev: &UdevEngineDevice) -> Option<StratisInfo> {
    let initialized = dev.is_initialized();
    if !initialized {
        warn!("Found a udev entry for a device identified as a Stratis device, but udev also identified it as uninitialized, disregarding the device");
        return None;
    };

    match decide_ownership(dev) {
        Err(err) => {
            warn!("Could not determine ownership of a block device identified as a Stratis device by udev, disregarding the device: {}",
                  err);
            None
        }
        Ok(ownership) => match ownership {
            UdevOwnership::Stratis => process_stratis_device(dev),
            UdevOwnership::MultipathMember => None,
            _ => {
                warn!("udev enumeration identified this device as a Stratis block device but on further examination udev identifies it as a {}",
                      ownership);
                None
            }
        },
    }
    .map(|info| {
        info!("Stratis block device with {} discovered during initial search",
              info,
        );
        info
    })
}

/// Identify a block device in the context where a udev event has been
/// captured for some block device. Return None if the device does not
/// appear to be a Stratis device. Log at an appropriate level on all errors.
pub fn identify_block_device(event: &UdevEngineEvent) -> Option<DeviceInfo> {
    let initialized = event.device().is_initialized();
    if !initialized {
        debug!("Found a udev entry for a device identified as a block device, but udev also identified it as uninitialized, disregarding the device");
        return None;
    };

    match decide_ownership(event.device()) {
        Err(err) => {
            warn!(
                "Could not determine ownership of a udev block device, disregarding the device: {}",
                err
            );
            None
        }
        Ok(ownership) => match ownership {
            UdevOwnership::Stratis => {
                process_stratis_device(event.device()).map(DeviceInfo::Stratis)
            }
            UdevOwnership::Luks => process_luks_device(event.device()).map(DeviceInfo::Luks),
            _ => None,
        },
    }
    .map(|info| {
        debug!("Stratis block device with {} identified", info);
        info
    })
}

/// Retrieve all block devices that should be made use of by the
/// Stratis engine. This excludes Stratis block devices that appear to be
/// multipath members.
///
/// Includes a fallback path, which is used if no Stratis block devices are
/// found using the obvious udev property- and enumerator-based approach.
/// This fallback path is more expensive, because it must search all block
/// devices via udev rather than just all Stratis block devices.
///
/// Omits any device that appears problematic in some way.
///
/// Return an error only on a failure to construct or scan with a udev
/// enumerator.
///
/// Returns a map of pool uuids to a map of devices to devnodes for each pool.
#[allow(clippy::type_complexity)]
pub fn find_all() -> libudev::Result<(
    HashMap<PoolUuid, Vec<LuksInfo>>,
    HashMap<PoolUuid, Vec<StratisInfo>>,
)> {
    info!("Beginning initial search for Stratis block devices");
    find_all_luks_devices()
        .and_then(|luks| find_all_stratis_devices().map(|stratis| (luks, stratis)))
}

#[cfg(test)]
mod tests {
    use crate::{
        engine::{
            strat_engine::{
                backstore::{initialize_devices, ProcessedPathInfos, UnownedDevices},
                cmd::create_fs,
                metadata::MDADataSize,
                tests::{crypt, loopbacked, real},
                udev::block_device_apply,
            },
            types::{DevicePath, EncryptionInfo, KeyDescription},
        },
        stratis::StratisResult,
    };

    use super::*;

    fn get_devices(paths: &[&Path]) -> StratisResult<UnownedDevices> {
        ProcessedPathInfos::try_from(paths)
            .map(|ps| ps.unpack())
            .map(|(sds, uds)| {
                sds.error_on_not_empty().unwrap();
                uds
            })
    }

    /// Test that an encrypted device initialized by stratisd is properly
    /// recognized.
    ///
    /// * Verify that the physical paths are recognized as LUKS devices
    /// belonging to Stratis.
    /// * Verify that the physical paths are not recognized as Stratis devices.
    /// * Verify that the metadata paths are recognized as Stratis devices.
    fn test_process_luks_device_initialized(paths: &[&Path]) {
        assert!(!paths.is_empty());

        fn luks_device_test(paths: &[&Path], key_description: &KeyDescription) {
            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());

            let devices = initialize_devices(
                get_devices(paths).unwrap(),
                pool_name,
                pool_uuid,
                MDADataSize::default(),
                Some(&EncryptionInfo::KeyDesc(key_description.clone())),
                None,
            )
            .unwrap();

            for dev in devices {
                let info =
                    block_device_apply(&DevicePath::new(dev.physical_path()).unwrap(), |dev| {
                        process_luks_device(dev)
                    })
                    .unwrap()
                    .expect("No device with specified devnode found in udev database")
                    .expect("No LUKS information for Stratis found on specified device");

                if info.identifiers.pool_uuid != pool_uuid {
                    panic!(
                        "Discovered pool UUID {} != expected pool UUID {}",
                        info.identifiers.pool_uuid, pool_uuid
                    );
                }

                if info.dev_info.devnode != dev.physical_path() {
                    panic!(
                        "Discovered device node {} != expected device node {}",
                        info.dev_info.devnode.display(),
                        dev.physical_path().display()
                    );
                }

                if info.encryption_info.key_description() != Some(key_description) {
                    panic!(
                        "Discovered key description {:?} != expected key description {:?}",
                        info.encryption_info.key_description(),
                        Some(key_description.as_application_str())
                    );
                }

                let info =
                    block_device_apply(&DevicePath::new(dev.physical_path()).unwrap(), |dev| {
                        process_stratis_device(dev)
                    })
                    .unwrap()
                    .expect("No device with specified devnode found in udev database");
                if info.is_some() {
                    panic!("Encrypted block device was incorrectly identified as a Stratis device");
                }

                let info =
                    block_device_apply(&DevicePath::new(dev.metadata_path()).unwrap(), |dev| {
                        process_stratis_device(dev)
                    })
                    .unwrap()
                    .expect("No device with specified devnode found in udev database")
                    .expect("No Stratis metadata found on specified device");

                if info.bda.identifiers().pool_uuid != pool_uuid
                    || info.dev_info.devnode != dev.metadata_path()
                {
                    panic!(
                        "Wrong identifiers and devnode found on Stratis block device: found: pool UUID: {}, device node; {} != expected: pool UUID: {}, device node: {}",
                        info.bda.identifiers().pool_uuid,
                        info.dev_info.devnode.display(),
                        pool_uuid,
                        dev.metadata_path().display(),
                    );
                }
            }
        }

        crypt::insert_and_cleanup_key(paths, luks_device_test);
    }

    #[test]
    fn loop_test_process_luks_device_initialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_process_luks_device_initialized,
        );
    }

    #[test]
    fn real_test_process_luks_device_initialized() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_process_luks_device_initialized,
        );
    }

    /// Test that the process_*_device methods return the expected
    /// pool UUID and device node for initialized paths.
    fn test_process_device_initialized(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let pool_uuid = PoolUuid::new_v4();
        let pool_name = Name::new("pool_name".to_string());

        initialize_devices(
            get_devices(paths).unwrap(),
            pool_name,
            pool_uuid,
            MDADataSize::default(),
            None,
            None,
        )
        .unwrap();

        for path in paths {
            let device_path = DevicePath::new(path).expect("our test path");
            let info = block_device_apply(&device_path, process_stratis_device)
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(info.bda.identifiers().pool_uuid, pool_uuid);
            assert_eq!(&&info.dev_info.devnode, path);

            assert_eq!(
                block_device_apply(&device_path, process_luks_device)
                    .unwrap()
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn loop_test_process_device_initialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_process_device_initialized,
        );
    }

    #[test]
    fn real_test_process_device_initialized() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_process_device_initialized,
        );
    }

    /// Test that the process_*_device methods return None if the device is
    /// not a Stratis device. Strictly speaking, the methods are only supposed
    /// to be called in particular contexts, the situation where the device
    /// is claimed by a filesystem should be excluded by udev, which should
    /// identify the device as Theirs. But the methods should return the
    /// correct result in this situation, regardless, although their log
    /// messages will not precisely match their actual situation, but rather
    /// their expected context.
    fn test_process_device_uninitialized(paths: &[&Path]) {
        assert!(!paths.is_empty());

        for path in paths {
            let device_path = DevicePath::new(path).expect("our test path");
            assert_eq!(
                block_device_apply(&device_path, process_stratis_device)
                    .unwrap()
                    .unwrap(),
                None
            );
            assert_eq!(
                block_device_apply(&device_path, process_luks_device)
                    .unwrap()
                    .unwrap(),
                None
            );
        }

        for path in paths {
            create_fs(path, None, false).unwrap();
            let device_path = DevicePath::new(path).expect("our test path");
            assert_eq!(
                block_device_apply(&device_path, process_stratis_device)
                    .unwrap()
                    .unwrap(),
                None
            );
            assert_eq!(
                block_device_apply(&device_path, process_luks_device)
                    .unwrap()
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn loop_test_process_device_uninitialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_process_device_uninitialized,
        );
    }

    #[test]
    fn real_test_process_device_uninitialized() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_process_device_uninitialized,
        );
    }
}
