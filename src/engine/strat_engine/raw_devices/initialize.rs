// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Initialization of Stratis devices

use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::Utc;
use uuid::Uuid;

use devicemapper::{Device, Sectors};

use crate::{
    engine::{
        strat_engine::{
            backstore::{wipe_blockdevs, StratBlockDev},
            metadata::{disown_device, BlockdevSize, MDADataSize, StratisIdentifiers, BDA},
            names::KeyDescription,
            raw_devices::{
                crypt::{CryptHandle, CryptInitializer},
                devices::InitDeviceInfo,
            },
        },
        types::{BlockDevPath, DevUuid, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

/// A handle to a device that may or may not be encrypted. This handle
/// contains the necessary information for writing metadata to both
/// encrypted and unencrypted devices.
enum MaybeEncrypted {
    Encrypted(CryptHandle),
    Unencrypted(PathBuf, PoolUuid),
}

impl MaybeEncrypted {
    /// This method returns the appropriate path information for both
    /// encrypted and unencrypted devices.
    ///
    /// * For encrypted devices, this will return the physical device path
    ///   for devicemapper operations and the logical path for writing metadata.
    /// * For unencrypted devices, this will return the physical path for
    ///   writing metadata.
    ///
    /// The returned error is a `StratisError` to limit usage of
    /// libcryptsetup-rs outside of the crypt module.
    fn to_blockdev_path(&self) -> StratisResult<BlockDevPath> {
        match *self {
            MaybeEncrypted::Unencrypted(ref path, _) => {
                Ok(BlockDevPath::physical_device_path(path))
            }
            MaybeEncrypted::Encrypted(ref handle) => {
                let physical = handle.physical_device_path();
                let logical = handle.logical_device_path().ok_or_else(|| {
                    StratisError::Error(
                        "Path required for writing Stratis metadata on an \
                            encrypted device does not exist"
                            .to_string(),
                    )
                })?;
                Ok(BlockDevPath::mapped_device_path(physical, &logical)?)
            }
        }
    }
}

/// Initialze devices in devices.
/// Clean up previously initialized devices if initialization of any single
/// device fails during initialization. Log at the warning level if cleanup
/// fails.
///
/// Precondition: All devices have been identified as ready to be initialized
/// in a previous step.
///
/// Precondition: Each device's InitDeviceInfo struct contains all necessary
/// information about the device.
pub fn initialize_devices(
    devices: Vec<InitDeviceInfo>,
    pool_uuid: PoolUuid,
    mda_data_size: MDADataSize,
    key_description: Option<&KeyDescription>,
) -> StratisResult<Vec<StratBlockDev>> {
    /// Map a major/minor device number of a physical device
    /// to the corresponding major/minor number of the encrypted
    /// device that uses the physical device as storage.
    fn map_device_nums(logical_path: &Path) -> StratisResult<Device> {
        let result = nix::sys::stat::stat(logical_path)?;
        Ok(Device::from(result.st_rdev))
    }

    /// Initialize an encrypted device on the given physical device
    /// using the pool and device UUIDs of the new Stratis block device
    /// and the key description for the key to use for encrypting the
    /// data.
    ///
    /// On failure, this method will roll back the initialization
    /// process and clean up the device that it has just initialized.
    fn initialize_encrypted(
        physical_path: &Path,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        key_description: &KeyDescription,
    ) -> StratisResult<(CryptHandle, Device, Sectors)> {
        let mut handle = CryptInitializer::new(physical_path.to_owned(), pool_uuid, dev_uuid)
            .initialize(key_description)?;
        let device_size = handle.logical_device_size()?;

        map_device_nums(
            &handle
                .logical_device_path()
                .expect("Initialization completed successfully"),
        )
        .map(|dn| (handle, dn, device_size))
    }

    fn initialize_stratis_metadata(
        path: &BlockDevPath,
        devno: Device,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        sizes: (MDADataSize, BlockdevSize),
        id_wwn: &Option<StratisResult<String>>,
        key_description: Option<&KeyDescription>,
    ) -> StratisResult<StratBlockDev> {
        let (mda_data_size, data_size) = sizes;
        let mut f = OpenOptions::new().write(true).open(path.metadata_path())?;

        // NOTE: Encrypted devices will discard the hardware ID as encrypted devices
        // are always represented as logical, software-based devicemapper devices
        // which will never have a hardware ID.
        let hw_id = match (key_description.is_some(), id_wwn) {
            (true, _) => None,
            (_, Some(Ok(ref hw_id))) => Some(hw_id.to_owned()),
            (_, Some(Err(_))) => {
                warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                      path.physical_path().display(),
                      pool_uuid.to_simple_ref());
                None
            }
            (_, None) => None,
        };

        let bda = BDA::initialize(
            &mut f,
            StratisIdentifiers::new(pool_uuid, dev_uuid),
            mda_data_size,
            data_size,
            Utc::now().timestamp() as u64,
        );

        bda.and_then(|bda| {
            StratBlockDev::new(
                devno,
                path.to_owned(),
                bda,
                &[],
                None,
                hw_id,
                key_description,
            )
        })
    }

    /// Clean up the Stratis metadata for encrypted andd unencrypted devices
    /// and log a warning if the device could not be cleaned up.
    fn clean_up(mut maybe_encrypted: MaybeEncrypted) {
        match maybe_encrypted {
            MaybeEncrypted::Encrypted(ref mut handle) => {
                if let Err(e) = handle.wipe() {
                    warn!(
                        "Failed to clean up encrypted device {}; cleanup \
                        was attempted because initialization of the device \
                        failed: {}",
                        handle.physical_device_path().display(),
                        e
                    );
                }
            }
            MaybeEncrypted::Unencrypted(ref physical_path, ref pool_uuid) => {
                if let Err(err) = OpenOptions::new()
                    .write(true)
                    .open(physical_path)
                    .map_err(StratisError::from)
                    .and_then(|mut f| disown_device(&mut f))
                {
                    warn!(
                        "Failed to clean up device {}; cleanup was attempted \
                        because initialization of the device for pool with \
                        UUID {} failed: {}",
                        physical_path.display(),
                        pool_uuid.to_simple_ref(),
                        err
                    );
                }
            }
        };
    }

    // Initialize a single device using information in dev_info.
    // If initialization fails at any stage clean up the device.
    // Return an error if initialization failed. Log a warning if cleanup
    // fails.
    //
    // This method will clean up after LUKS2 and unencrypted Stratis devices
    // in phases. In the case of encryption, if a device has been initialized
    // as an encrypted volume, it will either rely on StratBlockDev::disown()
    // if the in-memory StratBlockDev object has been created or
    // will call out directly to destroy_encrypted_stratis_device() if it
    // fails before that.
    fn initialize_one(
        dev_info: &InitDeviceInfo,
        pool_uuid: PoolUuid,
        mda_data_size: MDADataSize,
        key_description: Option<&KeyDescription>,
    ) -> StratisResult<StratBlockDev> {
        let dev_uuid = Uuid::new_v4();
        let (maybe_encrypted, devno, blockdev_size) = match key_description {
            Some(desc) => initialize_encrypted(&dev_info.devnode, pool_uuid, dev_uuid, desc).map(
                |(handle, devno, devsize)| {
                    debug!(
                        "Info on physical device {}, logical device {}",
                        &dev_info.devnode.display(),
                        handle
                            .logical_device_path()
                            .expect("Initialization must have succeeded")
                            .display(),
                    );
                    debug!(
                        "Physical device size: {}, logical device size: {}",
                        dev_info.size,
                        devsize.bytes(),
                    );
                    debug!(
                        "Physical device numbers: {}, logical device numbers: {}",
                        dev_info.devno, devno,
                    );
                    (MaybeEncrypted::Encrypted(handle), devno, devsize)
                },
            )?,
            None => (
                MaybeEncrypted::Unencrypted(dev_info.devnode.clone(), pool_uuid),
                dev_info.devno,
                dev_info.size.sectors(),
            ),
        };

        let path = match maybe_encrypted.to_blockdev_path() {
            Ok(p) => p,
            Err(e) => {
                clean_up(maybe_encrypted);
                return Err(e);
            }
        };
        let blockdev = initialize_stratis_metadata(
            &path,
            devno,
            pool_uuid,
            dev_uuid,
            (mda_data_size, BlockdevSize::new(blockdev_size)),
            &dev_info.id_wwn,
            key_description,
        );
        if blockdev.is_err() {
            clean_up(maybe_encrypted);
        }
        blockdev
    }

    let mut initialized_blockdevs: Vec<StratBlockDev> = Vec::new();
    for dev_info in devices {
        match initialize_one(
            &dev_info,
            pool_uuid,
            mda_data_size,
            key_description.as_deref(),
        ) {
            Ok(blockdev) => initialized_blockdevs.push(blockdev),
            Err(err) => {
                if let Err(err) = wipe_blockdevs(&initialized_blockdevs) {
                    warn!("Failed to clean up some devices after initialization of device {} for pool with UUID {} failed: {}",
                          dev_info.devnode.display(),
                          pool_uuid.to_simple_ref(),
                          err);
                }
                return Err(err);
            }
        }
    }
    Ok(initialized_blockdevs)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, error::Error, fs::OpenOptions};

    use uuid::Uuid;

    use crate::engine::strat_engine::{
        metadata::device_identifiers,
        raw_devices::{crypt::CryptHandle, devices::process_and_verify_devices},
        tests::{crypt, loopbacked, real},
    };

    use super::*;

    /// Test that initializing devices claims all and that destroying
    /// them releases all. Verify that already initialized devices are
    /// rejected or filtered as appropriate.
    fn test_ownership(
        paths: &[&Path],
        key_description: Option<&KeyDescription>,
    ) -> Result<(), Box<dyn Error>> {
        let pool_uuid = Uuid::new_v4();
        let infos: Vec<_> = process_and_verify_devices(pool_uuid, &HashSet::new(), paths)?;

        let blockdevs =
            initialize_devices(infos, pool_uuid, MDADataSize::default(), key_description)?;

        if blockdevs.len() != paths.len() {
            return Err(Box::new(StratisError::Error(
                "Fewer blockdevices were created than were requested".to_string(),
            )));
        }

        let stratis_devnodes: Vec<PathBuf> = blockdevs
            .iter()
            .map(|bd| bd.devnode().metadata_path().to_owned())
            .collect();

        let stratis_identifiers: Vec<Option<StratisIdentifiers>> = stratis_devnodes
            .iter()
            .map(|dev| {
                OpenOptions::new()
                    .read(true)
                    .open(&dev)
                    .map_err(|err| err.into())
                    .and_then(|mut f| device_identifiers(&mut f))
            })
            .collect::<StratisResult<Vec<Option<StratisIdentifiers>>>>()?;

        if stratis_identifiers.iter().any(Option::is_none) {
            return Err(Box::new(StratisError::Error(
                "Some device which should have had Stratis identifiers on it did not".to_string(),
            )));
        }

        if stratis_identifiers
            .iter()
            .any(|x| x.expect("returned in line above if any are None").pool_uuid != pool_uuid)
        {
            return Err(Box::new(StratisError::Error(
                "Some device had the wrong pool UUID".to_string(),
            )));
        }

        let initialized_uuids: HashSet<DevUuid> = stratis_identifiers
            .iter()
            .map(|ids| {
                ids.expect("returned in line above if any are None")
                    .device_uuid
            })
            .collect();

        if !process_and_verify_devices(
            pool_uuid,
            &initialized_uuids,
            stratis_devnodes
                .iter()
                .map(|p| p.as_path())
                .collect::<Vec<_>>()
                .as_slice(),
        )?
        .is_empty()
        {
            return Err(Box::new(StratisError::Error(
                "Failed to eliminate devices already initialized for this pool from list of devices to initialize".to_string()
            )));
        }

        if process_and_verify_devices(
            pool_uuid,
            &HashSet::new(),
            stratis_devnodes
                .iter()
                .map(|p| p.as_path())
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .is_ok()
        {
            return Err(Box::new(StratisError::Error(
                "Failed to return an error when some device processed was not in the set of already initialized devices".to_string()
            )));
        }

        if process_and_verify_devices(
            Uuid::new_v4(),
            &initialized_uuids,
            stratis_devnodes
                .iter()
                .map(|p| p.as_path())
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .is_ok()
        {
            return Err(Box::new(StratisError::Error(
                "Failed to return an error when processing devices for a pool UUID which is not the same as that for which the devices were initialized".to_string()
            )));
        }

        let result = process_and_verify_devices(pool_uuid, &initialized_uuids, paths);
        if key_description.is_some() && result.is_ok() {
            return Err(Box::new(StratisError::Error(
                "Failed to return an error when encountering devices that are LUKS2".to_string(),
            )));
        }

        if key_description.is_none() && !result?.is_empty() {
            return Err(Box::new(StratisError::Error(
                        "Failed to filter all previously initialized devices which should have all been eliminated on the basis of already belonging to pool with the given pool UUID".to_string()
                )));
        }

        wipe_blockdevs(&blockdevs)?;

        for path in paths {
            if key_description.is_some() {
                if CryptHandle::setup(path)?.is_some() {
                    return Err(Box::new(StratisError::Error(
                        "LUKS2 metadata on Stratis devices was not successfully wiped".to_string(),
                    )));
                }
            } else if device_identifiers(&mut OpenOptions::new().read(true).open(path)?)? != None {
                return Err(Box::new(StratisError::Error(
                    "Metadata on Stratis devices was not successfully wiped".to_string(),
                )));
            }
        }
        Ok(())
    }

    /// Test ownership with encryption
    fn test_ownership_crypt(paths: &[&Path]) {
        fn call_crypt_test(
            paths: &[&Path],
            key_description: &KeyDescription,
            _: Option<()>,
        ) -> Result<(), Box<dyn Error>> {
            test_ownership(paths, Some(key_description))
        }

        crypt::insert_and_cleanup_key(paths, call_crypt_test)
    }

    /// Test ownership with no encryption
    fn test_ownership_no_crypt(paths: &[&Path]) {
        test_ownership(paths, None).unwrap()
    }

    #[test]
    fn loop_test_ownership() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_ownership_no_crypt,
        );
    }

    #[test]
    fn real_test_ownership() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_ownership_no_crypt,
        );
    }

    #[test]
    fn travis_test_ownership() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_ownership_no_crypt,
        );
    }

    #[test]
    fn loop_test_crypt_ownership() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_ownership_crypt,
        );
    }

    #[test]
    fn real_test_crypt_ownership() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_ownership_crypt,
        );
    }

    #[test]
    fn travis_test_crypt_ownership() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_ownership_crypt,
        );
    }

    // Verify that if the last device in a list of devices to initialize
    // can not be initialized, all the devices previously initialized are
    // properly cleaned up.
    fn test_failure_cleanup(
        paths: &[&Path],
        key_desc: Option<&KeyDescription>,
    ) -> Result<(), Box<dyn Error>> {
        if paths.len() <= 1 {
            return Err(Box::new(StratisError::Error(
                "Test requires more than one device".to_string(),
            )));
        }

        let pool_uuid = Uuid::new_v4();
        let mut infos: Vec<_> = process_and_verify_devices(pool_uuid, &HashSet::new(), paths)?;

        // Synthesize a InitDeviceInfo that will cause initialization to fail.
        {
            let old_info = infos.pop().expect("Must contain at least two devices");

            let new_info = InitDeviceInfo {
                devnode: PathBuf::from("/srk/cheese"),
                devno: old_info.devno,
                id_wwn: None,
                size: old_info.size,
            };

            infos.push(new_info);
        }

        if initialize_devices(infos, pool_uuid, MDADataSize::default(), key_desc).is_ok() {
            return Err(Box::new(StratisError::Error(
                "Initialization should not have succeeded".to_string(),
            )));
        }

        // Check all paths for absence of device identifiers or LUKS2 metadata
        // depending on whether or not it is encrypted. Initialization of the
        // last path was never attempted, so it should be as bare of Stratis
        // identifiers as all the other paths that were initialized.
        for path in paths {
            if key_desc.is_some() {
                if CryptHandle::setup(path)?.is_some() {
                    return Err(Box::new(StratisError::Error(format!(
                        "Device {} should have no LUKS2 metadata",
                        path.display()
                    ))));
                }
            } else {
                let mut f = OpenOptions::new().read(true).write(true).open(path)?;
                match device_identifiers(&mut f) {
                    Ok(None) => (),
                    _ => {
                        return Err(Box::new(StratisError::Error(format!(
                            "Device {} should have returned nothing for device identifiers",
                            path.display()
                        ))))
                    }
                }
            }
        }
        Ok(())
    }

    // Run test_failure_cleanup for encrypted devices
    fn test_failure_cleanup_crypt(paths: &[&Path]) {
        fn failure_cleanup_crypt(
            paths: &[&Path],
            key_desc: &KeyDescription,
            _: Option<()>,
        ) -> Result<(), Box<dyn Error>> {
            test_failure_cleanup(paths, Some(key_desc))
        }

        crypt::insert_and_cleanup_key(paths, failure_cleanup_crypt)
    }

    // Run test_failure_cleanup for unencrypted devices
    fn test_failure_cleanup_no_crypt(paths: &[&Path]) {
        test_failure_cleanup(paths, None).unwrap()
    }

    #[test]
    fn loop_test_crypt_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup_crypt,
        );
    }

    #[test]
    fn real_test_crypt_failure_cleanup() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_failure_cleanup_crypt,
        );
    }

    #[test]
    fn travis_test_crypt_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup_crypt,
        );
    }

    #[test]
    fn loop_test_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup_no_crypt,
        );
    }

    #[test]
    fn real_test_failure_cleanup() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(2, None, None),
            test_failure_cleanup_no_crypt,
        );
    }

    #[test]
    fn travis_test_failure_cleanup() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(2, 3, None),
            test_failure_cleanup_no_crypt,
        );
    }
}
