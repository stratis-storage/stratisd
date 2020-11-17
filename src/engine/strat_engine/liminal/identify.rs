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
//! They have the following invocation heirarchy:
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
    path::{Path, PathBuf},
};

use serde_json::Value;

use devicemapper::Device;

use crate::engine::{
    strat_engine::{
        backstore::CryptHandle,
        metadata::{device_identifiers, StratisIdentifiers},
        udev::{
            block_enumerator, decide_ownership, UdevOwnership, CRYPTO_FS_TYPE, FS_TYPE_KEY,
            STRATIS_FS_TYPE,
        },
    },
    types::{EncryptionInfo, KeyDescription, PoolUuid},
};

/// A miscellaneous group of identifiers found when identifying a LUKS
/// device which belongs to Stratis.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LuksInfo {
    /// All the usual StratisInfo
    pub info: StratisInfo,
    /// Encryption information
    pub encryption_info: EncryptionInfo,
}

impl fmt::Display for LuksInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}, {}", self.info, self.encryption_info,)
    }
}

/// A miscellaneous group of identifiers found when identifying a Stratis
/// device.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StratisInfo {
    pub identifiers: StratisIdentifiers,
    pub device_number: Device,
    pub devnode: PathBuf,
}

impl fmt::Display for StratisInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}, device number: \"{}\", devnode: \"{}\"",
            self.identifiers,
            self.device_number,
            self.devnode.display()
        )
    }
}

impl<'a> Into<Value> for &'a StratisInfo {
    // Precondition: (&StratisIdentifiers).into() pattern matches
    // Value::Object()
    fn into(self) -> Value {
        let mut json = json!({
            "major": Value::from(self.device_number.major),
            "minor": Value::from(self.device_number.minor),
            "devnode": Value::from(self.devnode.display().to_string())
        });
        if let Value::Object(ref mut map) = json {
            map.extend(
                if let Value::Object(map) =
                    <&StratisIdentifiers as Into<Value>>::into(&self.identifiers)
                {
                    map.into_iter()
                } else {
                    unreachable!("StratisIdentifiers conversion returns a JSON object");
                },
            );
        } else {
            unreachable!("json!() always creates a JSON object")
        };
        json
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
            DeviceInfo::Luks(info) => info.info.identifiers,
            DeviceInfo::Stratis(info) => info.identifiers,
        }
    }

    pub fn key_description(&self) -> Option<&KeyDescription> {
        match self {
            DeviceInfo::Luks(info) => Some(&info.encryption_info.key_description),
            DeviceInfo::Stratis(_) => None,
        }
    }
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DeviceInfo::Luks(info) => write!(f, "LUKS device description: {}", info),
            DeviceInfo::Stratis(info) => write!(f, "Stratis device description: {}", info),
        }
    }
}

// A wrapper for obtaining the device number as a devicemapper Device
// which interprets absence of the value as an error, which it is in this
// context.
fn device_to_devno_wrapper(device: &libudev::Device) -> Result<Device, String> {
    device
        .devnum()
        .ok_or_else(|| "udev entry did not contain a device number".into())
        .map(Device::from)
}

// A wrapper around the metadata module's device_identifers method
// which also handles failure to open a device for reading.
// Returns an error if the device could not be opened for reading.
// Returns Ok(Err(...)) if there was an error while reading the
// Stratis identifiers from the device.
// Returns Ok(Ok(None)) if the identifers did not appear to be on
// the device.
fn device_identifiers_wrapper(
    devnode: &Path,
) -> Result<Result<Option<StratisIdentifiers>, String>, String> {
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
            device_identifiers(f).map_err(|err| {
                format!(
                    "encountered an error while reading Stratis header for device {}: {}",
                    devnode.display(),
                    err
                )
            })
        })
}

/// Process a device which udev information indicates is a LUKS device.
fn process_luks_device(dev: &libudev::Device) -> Option<LuksInfo> {
    match dev.devnode() {
        Some(devnode) => match device_to_devno_wrapper(dev) {
            Err(err) => {
                warn!(
                    "udev identified device {} as a Stratis device but {}, disregarding the device",
                    devnode.display(),
                    err
                );
                None
            }
            Ok(device_number) => match CryptHandle::setup(devnode) {
                Ok(None) => None,
                Err(err) => {
                    warn!(
                            "udev identified device {} as a LUKS device, but could not read LUKS header from the device, disregarding the device: {}",
                            devnode.display(),
                            err,
                            );
                    None
                }
                Ok(Some(mut handle)) => match handle.clevis_info() {
                    Ok(clevis_info) => Some(LuksInfo {
                        info: StratisInfo {
                            identifiers: *handle.device_identifiers(),
                            device_number,
                            devnode: handle.physical_device_path().to_path_buf(),
                        },
                        encryption_info: EncryptionInfo {
                            key_description: handle.key_description().clone(),
                            clevis_info,
                        },
                    }),
                    Err(err) => {
                        warn!(
                                "There was a problem decoding the Clevis info on device {}, disregarding the device: {}",
                                devnode.display(),
                                err
                                );
                        None
                    }
                },
            },
        },
        None => {
            warn!("udev identified a device as a LUKS2 device, but the udev entry for the device had no device node, disregarding device");
            None
        }
    }
}

/// Process a device which udev information indicates is a Stratis device.
fn process_stratis_device(dev: &libudev::Device) -> Option<StratisInfo> {
    match dev.devnode() {
        Some(devnode) => {
            match (
                device_to_devno_wrapper(dev),
                device_identifiers_wrapper(devnode),
            ) {
                (Err(err), _) | (_, Err(err)) | (_, Ok(Err(err))) => {
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
                (Ok(device_number), Ok(Ok(Some(identifiers)))) => Some(StratisInfo {
                    identifiers,
                    device_number,
                    devnode: devnode.to_path_buf(),
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
        .filter_map(|dev| identify_luks_device(&dev))
        .fold(HashMap::new(), |mut acc, info| {
            acc.entry(info.info.identifiers.pool_uuid)
                .or_insert_with(Vec::new)
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
        .filter_map(|dev| identify_stratis_device(&dev))
        .fold(HashMap::new(), |mut acc, info| {
            acc.entry(info.identifiers.pool_uuid)
                .or_insert_with(Vec::new)
                .push(info);
            acc
        });
    Ok(pool_map)
}

// Identify a device that udev enumeration has already picked up as a LUKS
// device. Return None if the device does not, after all, appear to be a LUKS
// device belonging to Stratis. Log anything unusual at an appropriate level.
fn identify_luks_device(dev: &libudev::Device) -> Option<LuksInfo> {
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
fn identify_stratis_device(dev: &libudev::Device) -> Option<StratisInfo> {
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
pub fn identify_block_device(dev: &libudev::Device) -> Option<DeviceInfo> {
    let initialized = dev.is_initialized();
    if !initialized {
        debug!("Found a udev entry for a device identified as a block device, but udev also identified it as uninitialized, disregarding the device");
        return None;
    };

    match decide_ownership(dev) {
        Err(err) => {
            warn!(
                "Could not determine ownership of a udev block device, disregarding the device: {}",
                err
            );
            None
        }
        Ok(ownership) => match ownership {
            UdevOwnership::Stratis => process_stratis_device(dev).map(DeviceInfo::Stratis),
            UdevOwnership::Luks => process_luks_device(dev).map(DeviceInfo::Luks),
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

    use std::{collections::HashSet, error::Error};

    use uuid::Uuid;

    use crate::{
        engine::{
            strat_engine::{
                backstore::{initialize_devices, process_and_verify_devices},
                cmd::create_fs,
                metadata::MDADataSize,
                tests::{crypt, loopbacked, real},
                udev::block_device_apply,
            },
            types::EncryptionInfo,
        },
        stratis::StratisError,
    };

    use super::*;

    /// Test that an encrypted device initialized by stratisd is properly
    /// recognized.
    ///
    /// * Verify that the physical paths are recognized as LUKS devices
    /// belonging to Stratis.
    /// * Verify that the physical paths are not recognized as Stratis devices.
    /// * Verify that the metadata paths are recognized as Stratis devices.
    fn test_process_luks_device_initialized(paths: &[&Path]) {
        assert!(!paths.is_empty());

        fn luks_device_test(
            paths: &[&Path],
            key_description: &KeyDescription,
            _: Option<()>,
        ) -> Result<(), Box<dyn Error>> {
            let pool_uuid = Uuid::new_v4();

            let devices = initialize_devices(
                process_and_verify_devices(pool_uuid, &HashSet::new(), paths)?,
                pool_uuid,
                MDADataSize::default(),
                Some(EncryptionInfo {
                    key_description: key_description.clone(),
                    clevis_info: None,
                }),
            )?;

            for devnode in devices.iter().map(|sbd| sbd.devnode()) {
                let info =
                    block_device_apply(devnode.physical_path(), |dev| process_luks_device(dev))?
                        .ok_or_else(|| {
                            StratisError::Error(
                                "No device with specified devnode found in udev database".into(),
                            )
                        })?
                        .ok_or_else(|| {
                            StratisError::Error(
                                "No LUKS information for Stratis found on specified device".into(),
                            )
                        })?;

                if info.info.identifiers.pool_uuid != pool_uuid {
                    return Err(Box::new(StratisError::Error(format!(
                        "Discovered pool UUID {} != expected pool UUID {}",
                        info.info.identifiers.pool_uuid.to_simple_ref(),
                        pool_uuid.to_simple_ref()
                    ))));
                }

                if info.info.devnode != devnode.physical_path() {
                    return Err(Box::new(StratisError::Error(format!(
                        "Discovered device node {} != expected device node {}",
                        info.info.devnode.display(),
                        devnode.physical_path().display()
                    ))));
                }

                if &info.encryption_info.key_description != key_description {
                    return Err(Box::new(StratisError::Error(format!(
                        "Discovered key description {} != expected key description {}",
                        info.encryption_info.key_description.as_application_str(),
                        key_description.as_application_str()
                    ))));
                }

                let info =
                    block_device_apply(devnode.physical_path(), |dev| process_stratis_device(dev))?
                        .ok_or_else(|| {
                            StratisError::Error(
                                "No device with specified devnode found in udev database".into(),
                            )
                        })?;
                if info.is_some() {
                    return Err(Box::new(StratisError::Error(
                        "Encrypted block device was incorrectly identified as a Stratis device"
                            .to_string(),
                    )));
                }

                let info =
                    block_device_apply(devnode.user_path(), |dev| process_stratis_device(dev))?
                        .ok_or_else(|| {
                            StratisError::Error(
                                "No device with specified devnode found in udev database".into(),
                            )
                        })?
                        .ok_or_else(|| {
                            StratisError::Error(
                                "No Stratis metadata found on specified device".into(),
                            )
                        })?;

                if info.identifiers.pool_uuid != pool_uuid || info.devnode != devnode.user_path() {
                    return Err(Box::new(StratisError::Error(format!(
                        "Wrong identifiers and devnode found on Stratis block device: found: pool UUID: {}, device node; {} != expected: pool UUID: {}, device node: {}",
                        info.identifiers.pool_uuid.to_simple_ref(),
                        info.devnode.display(),
                        pool_uuid,
                        devnode.metadata_path().display()),
                    )));
                }
            }
            Ok(())
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

    #[test]
    fn travis_test_process_luks_device_initialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_process_luks_device_initialized,
        );
    }

    /// Test that the process_*_device methods return the expected
    /// pool UUID and device node for initialized paths.
    fn test_process_device_initialized(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let pool_uuid = Uuid::new_v4();

        initialize_devices(
            process_and_verify_devices(pool_uuid, &HashSet::new(), paths).unwrap(),
            pool_uuid,
            MDADataSize::default(),
            None,
        )
        .unwrap();

        for path in paths {
            let info = block_device_apply(path, |dev| process_stratis_device(dev))
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(info.identifiers.pool_uuid, pool_uuid);
            assert_eq!(&&info.devnode, path);

            assert_eq!(
                block_device_apply(path, |dev| process_luks_device(dev))
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

    #[test]
    fn travis_test_process_device_initialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
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
            assert_eq!(
                block_device_apply(path, |dev| process_stratis_device(dev))
                    .unwrap()
                    .unwrap(),
                None
            );
            assert_eq!(
                block_device_apply(path, |dev| process_luks_device(dev))
                    .unwrap()
                    .unwrap(),
                None
            );
        }

        for path in paths {
            create_fs(path, None).unwrap();
            assert_eq!(
                block_device_apply(path, |dev| process_stratis_device(dev))
                    .unwrap()
                    .unwrap(),
                None
            );
            assert_eq!(
                block_device_apply(path, |dev| process_luks_device(dev))
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

    #[test]
    fn travis_test_process_device_uninitialized() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_process_device_uninitialized,
        );
    }
}
