// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use chrono::Utc;
use itertools::Itertools;
use uuid::Uuid;

use devicemapper::{Bytes, Device, Sectors, IEC};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::StratBlockDev,
                crypt::{CryptHandle, CryptInitializer},
            },
            device::blkdev_size,
            metadata::{
                device_identifiers, disown_device, BlockdevSize, MDADataSize, StratisIdentifiers,
                BDA,
            },
            names::KeyDescription,
            udev::{block_device_apply, decide_ownership, get_udev_property, UdevOwnership},
        },
        types::{BlockDevPath, DevUuid, PoolUuid},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi as u128);

// Get information that can be obtained from udev for the block device
// identified by devnode. Return an error if there was an error finding the
// information or no udev entry corresponding to the devnode could be found.
// Return an error if udev ownership could not be obtained.
fn udev_info(
    devnode: &Path,
) -> StratisResult<(UdevOwnership, Device, Option<StratisResult<String>>)> {
    block_device_apply(devnode, |d| {
        (
            decide_ownership(d),
            d.devnum(),
            get_udev_property(d, "ID_WWN"),
        )
    })
    .and_then(|res| {
        res.ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "Block device {} could not be found in the udev database",
                    devnode.display()
                ),
            )
        })
    })
    .map_err(|err| {
        StratisError::Engine(
            ErrorEnum::NotFound,
            format!(
                "Could not obtain udev information for block device {}: {}",
                devnode.display(),
                err
            ),
        )
    })
    .and_then(|(ownership, devnum, id_wwn)| {
        devnum
            .ok_or_else(|| {
                StratisError::Error(format!(
                    "Insufficient information: no device number found for device {} using udev",
                    devnode.display()
                ))
            })
            .map(|dev| (ownership, Device::from(dev), id_wwn))
    })
    .and_then(|(ownership, devnum, id_wwn)| {
        ownership
            .map(|ownership| (ownership, devnum, id_wwn))
            .map_err(|err| {
                StratisError::Error(format!(
                    "Could not obtain ownership information for device {} using udev: {}",
                    devnode.display(),
                    err
                ))
            })
    })
}

// Find information from the devnode that is useful to identify a device or
// to construct a StratBlockDev object. Returns a tuple of the ID_WWN,
// the size of the device, and Stratis identifiers for the device, if any
// are found. If the value for the Stratis identifiers is None, then this
// device has been determined to be unowned.
#[allow(clippy::type_complexity)]
fn dev_info(
    devnode: &Path,
) -> StratisResult<(
    Option<StratisResult<String>>,
    Bytes,
    Option<StratisIdentifiers>,
    Device,
)> {
    let (ownership, devnum, hw_id) = udev_info(devnode)?;
    match ownership {
        UdevOwnership::Luks | UdevOwnership::MultipathMember | UdevOwnership::Theirs => {
            let err_str = format!(
                "udev information indicates that device {} is a {}",
                devnode.display(),
                ownership
            );
            Err(StratisError::Engine(ErrorEnum::Invalid, err_str))
        }
        UdevOwnership::Stratis | UdevOwnership::Unowned => {
            let mut f = OpenOptions::new().read(true).write(true).open(&devnode)?;
            let dev_size = blkdev_size(&f)?;

            // FIXME: Read device identifiers from either an Unowned or a
            // Stratis device. For a Stratis device, this is the correct thing
            // to do. For an unowned device, this is the best available check
            // that we currently have to prevent overwriting a device which
            // is owned, but which udev has not identified as such. In future,
            // we hope to use libblkid in order to double check that the
            // device is truly unowned, not just for Stratis but also for
            // other potential owners.
            let stratis_identifiers = device_identifiers(&mut f).map_err(|err| {
                let error_message = format!(
                    "There was an error reading Stratis metadata from device {}; the device is unsuitable for initialization: {}",
                    devnode.display(),
                    err
                );
                StratisError::Engine(ErrorEnum::Invalid, error_message)
            })?;

            if ownership == UdevOwnership::Stratis && stratis_identifiers.is_none() {
                let error_message = format!(
                    "udev identified device {} as a Stratis device but device metadata does not show that it is a Stratis device",
                    devnode.display()
                );
                return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
            }

            Ok((hw_id, dev_size, stratis_identifiers, devnum))
        }
    }
}

/// A miscellaneous grab bag of useful information required to decide whether
/// a device should be allowed to be initialized by Stratis or to be used
/// when initializing a device.
#[derive(Debug)]
pub struct DeviceInfo {
    /// The device number
    pub devno: Device,
    /// The devnode
    pub devnode: PathBuf,
    /// The ID_WWN udev value, if present, supposed to uniquely identify the
    /// device.
    pub id_wwn: Option<StratisResult<String>>,
    /// The total size of the device
    pub size: Bytes,
}

// Process a list of devices specified as device nodes.
//
// * Reduce the list of devices to a set.
// * Return a vector of accumulated information about the device nodes.
//
// If the StratisIdentifiers value is not None, then the device has been
// identified as a Stratis device.
//
// Return an error if there was an error collecting the information or
// if it turns out that at least two of the specified devices have the same
// device number.
fn process_devices(
    paths: &[&Path],
) -> StratisResult<Vec<(DeviceInfo, Option<StratisIdentifiers>)>> {
    let infos = paths
        .iter()
        .unique()
        .map(|devnode| {
            dev_info(devnode).map(|(id_wwn, size, stratis_identifiers, devno)| {
                (
                    DeviceInfo {
                        devno,
                        devnode: devnode.to_path_buf(),
                        id_wwn,
                        size,
                    },
                    stratis_identifiers,
                )
            })
        })
        .collect::<StratisResult<Vec<(DeviceInfo, Option<StratisIdentifiers>)>>>()
        .map_err(|err| {
            let error_message = format!(
                "At least one of the devices specified was unsuitable for initialization: {}",
                err
            );
            StratisError::Engine(ErrorEnum::Invalid, error_message)
        })?;

    let duplicate_device_number_messages: Vec<String> = infos
        .iter()
        .map(|(info, _)| (info.devno, info.devnode.to_path_buf()))
        .fold(HashMap::new(), |mut acc, (devno, devnode)| {
            acc.entry(devno).or_insert_with(Vec::new).push(devnode);
            acc
        })
        .iter()
        .filter(|(_, devnodes)| devnodes.len() > 1)
        .map(|(devno, devnodes)| {
            format!(
                "device nodes {} correspond to device number {}",
                devnodes.iter().map(|d| d.display()).join(", "),
                devno
            )
        })
        .collect();

    if !duplicate_device_number_messages.is_empty() {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "At least two of the devices specified have the same device number: {}",
                duplicate_device_number_messages.join("; ")
            ),
        ));
    }

    Ok(infos)
}

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

// Check coherence of pool and device UUIDs against a set of current UUIDs.
// If the selection of devices is incompatible with the current
// state of the set, or simply invalid, return an error.
//
// Postcondition: There are no infos returned from this method that
// represent information about Stratis devices. If any device with
// Stratis identifiers was in the list of devices passed to the function
// either:
// * It was filtered out of the list, because its device UUID was
// found in current_uuids
// * An error was returned because it was unsuitable, for example,
// its pool UUID did not match the pool_uuid argument
//
// Precondition: This method is called only with the result of
// process_devices. Currently, this guarantees that all LUKS devices,
// for example, have been eliminated from the devices that are being
// checked. Thus, the absence of stratisd identifiers for a particular
// device should ensure that the device does not belong to Stratis.
//
// FIXME:
// Note that this method _should_ be somewhat temporary. We hope that in
// another step the functionality contained will be hoisted up closer to
// the D-Bus/engine interface, as it computes some idempotency information.
fn check_device_ids(
    pool_uuid: PoolUuid,
    current_uuids: &HashSet<DevUuid>,
    mut devices: Vec<(DeviceInfo, Option<StratisIdentifiers>)>,
) -> StratisResult<Vec<DeviceInfo>> {
    let (mut stratis_devices, mut non_stratis_devices) = (vec![], vec![]);

    for (info, ids) in devices.drain(..) {
        match ids {
            Some(ids) => stratis_devices.push((info, ids)),
            None => non_stratis_devices.push(info),
        }
    }

    let mut pools: HashMap<PoolUuid, Vec<(DevUuid, DeviceInfo)>> =
        stratis_devices
            .drain(..)
            .fold(HashMap::new(), |mut acc, (info, identifiers)| {
                acc.entry(identifiers.pool_uuid)
                    .or_insert_with(Vec::new)
                    .push((identifiers.device_uuid, info));
                acc
            });

    let this_pool: Option<Vec<(DevUuid, DeviceInfo)>> = pools.remove(&pool_uuid);

    if !pools.is_empty() {
        let error_string = pools
            .iter()
            .map(|(pool_uuid, devs)| {
                format!(
                    "devices ({}) appear to belong to Stratis pool with UUID {}",
                    devs.iter()
                        .map(|(_, info)| info.devnode.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    pool_uuid.to_simple_ref()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let error_message = format!(
            "Some devices specified appear to be already in use by other Stratis pools: {}",
            error_string
        );
        return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
    }

    if let Some(mut this_pool) = this_pool {
        let (mut included, mut not_included) = (vec![], vec![]);
        for (dev_uuid, info) in this_pool.drain(..) {
            if current_uuids.contains(&dev_uuid) {
                included.push((dev_uuid, info))
            } else {
                not_included.push((dev_uuid, info))
            }
        }

        if !not_included.is_empty() {
            let error_message = format!(
                "Devices ({}) appear to be already in use by this pool which has UUID {}; they may be in use by the other tier",
                not_included
                    .iter()
                    .map(|(_, info)| info.devnode.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                pool_uuid.to_simple_ref()
            );
            return Err(StratisError::Engine(ErrorEnum::Invalid, error_message));
        }

        if !included.is_empty() {
            info!(
                "Devices [{}] appear to be already in use by this pool which has UUID {}; omitting from the set of devices to initialize",
                included
                    .iter()
                    .map(|(dev_uuid, info)| {
                        format!(
                            "(device node: {}, device UUID: {})",
                            info.devnode.display().to_string(),
                            dev_uuid.to_simple_ref()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                pool_uuid.to_simple_ref()
            );
        }
    }

    Ok(non_stratis_devices)
}

/// Combine the functionality of process_devices and check_device_ids.
/// It is useful to guarantee that check_device_ids is called only with
/// the result of invoking process_devices.
pub fn process_and_verify_devices(
    pool_uuid: PoolUuid,
    current_uuids: &HashSet<DevUuid>,
    paths: &[&Path],
) -> StratisResult<Vec<DeviceInfo>> {
    check_device_ids(pool_uuid, current_uuids, process_devices(paths)?)
        .and_then(|vec| {
            vec
                .into_iter()
                .map(|info| {
                    if info.size < MIN_DEV_SIZE {
                        let error_message = format!(
                            "Device {} is {} which is smaller than the minimum required size for a Stratis blockdev, {}",
                            info.devnode.display(),
                            info.size,
                            MIN_DEV_SIZE);
                        Err(StratisError::Engine(ErrorEnum::Invalid, error_message))
                    } else { Ok(info) }
                })
                .collect()
        })
}

/// Initialze devices in devices.
/// Clean up previously initialized devices if initialization of any single
/// device fails during initialization. Log at the warning level if cleanup
/// fails.
///
/// Precondition: All devices have been identified as ready to be initialized
/// in a previous step.
///
/// Precondition: Each device's DeviceInfo struct contains all necessary
/// information about the device.
pub fn initialize_devices(
    devices: Vec<DeviceInfo>,
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
        dev_info: &DeviceInfo,
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

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &[StratBlockDev]) -> StratisResult<()> {
    let unerased_devnodes: Vec<_> = blockdevs
        .iter()
        .filter_map(|bd| match bd.disown() {
            Err(_) => Some(bd.devnode().physical_path()),
            _ => None,
        })
        .collect();

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let err_msg = format!(
            "Failed to wipe already initialized devnodes: {:?}",
            unerased_devnodes
        );
        Err(StratisError::Engine(ErrorEnum::Error, err_msg))
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs::OpenOptions};

    use uuid::Uuid;

    use crate::engine::strat_engine::{
        backstore::crypt::CryptHandle,
        metadata::device_identifiers,
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
        let infos: Vec<_> = process_devices(paths)?;

        if infos.len() != paths.len() {
            return Err(Box::new(StratisError::Error(
                "Some duplicate devices were found".to_string(),
            )));
        }

        let dev_infos = check_device_ids(pool_uuid, &HashSet::new(), infos)?;

        if dev_infos.len() != paths.len() {
            return Err(Box::new(StratisError::Error(
                "Some devices were filtered from the specified set".to_string(),
            )));
        }

        let blockdevs = initialize_devices(
            dev_infos,
            pool_uuid,
            MDADataSize::default(),
            key_description,
        )?;

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

    // Verify that a non-existent path results in a reasonably elegant
    // error, i.e., not an assertion failure.
    fn test_nonexistent_path(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let test_paths = [paths, &[Path::new("/srk/cheese")]].concat();

        assert_matches!(process_devices(&test_paths), Err(_));
    }

    #[test]
    fn loop_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
        );
    }

    #[test]
    fn real_test_nonexistent_path() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_nonexistent_path,
        );
    }

    #[test]
    fn travis_test_nonexistent_path() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_nonexistent_path,
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

        let infos: Vec<_> = process_devices(paths)?;
        let pool_uuid = Uuid::new_v4();

        if infos.len() != paths.len() {
            return Err(Box::new(StratisError::Error(
                "Some duplicate devices were found".to_string(),
            )));
        }

        let mut dev_infos = check_device_ids(pool_uuid, &HashSet::new(), infos)?;

        if dev_infos.len() != paths.len() {
            return Err(Box::new(StratisError::Error(
                "Some devices were filtered from the specified set".to_string(),
            )));
        }

        // Synthesize a DeviceInfo that will cause initialization to fail.
        {
            let old_info = dev_infos.pop().expect("Must contain at least two devices");

            let new_info = DeviceInfo {
                devnode: PathBuf::from("/srk/cheese"),
                devno: old_info.devno,
                id_wwn: None,
                size: old_info.size,
            };

            dev_infos.push(new_info);
        }

        if initialize_devices(dev_infos, pool_uuid, MDADataSize::default(), key_desc).is_ok() {
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

    // Verify that resolve devices simply eliminates duplicate devnodes,
    // without returning an error.
    fn test_duplicate_devnodes(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let duplicate_paths = paths
            .iter()
            .chain(paths.iter())
            .copied()
            .collect::<Vec<_>>();

        let result = process_devices(&duplicate_paths).unwrap();

        assert_eq!(result.len(), paths.len());
    }

    #[test]
    fn loop_test_duplicate_devnodes() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 2, None),
            test_duplicate_devnodes,
        );
    }

    #[test]
    fn real_test_duplicate_devnodes() {
        real::test_with_spec(
            &real::DeviceLimits::AtLeast(1, None, None),
            test_duplicate_devnodes,
        );
    }

    #[test]
    fn travis_test_duplicate_devnodes() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 2, None),
            test_duplicate_devnodes,
        );
    }
}
