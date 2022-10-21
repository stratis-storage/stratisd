// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::Utc;
use itertools::Itertools;
use nix::sys::stat::stat;

use devicemapper::{Bytes, Device, Sectors, IEC};
use libblkid_rs::{BlkidCache, BlkidProbe};

use crate::{
    engine::{
        strat_engine::{
            backstore::{
                blockdev::{StratBlockDev, UnderlyingDevice},
                crypt::{CryptHandle, CryptInitializer},
            },
            device::blkdev_size,
            metadata::{
                device_identifiers, disown_device, BlockdevSize, MDADataSize, StratisIdentifiers,
                BDA,
            },
            names::KeyDescription,
            udev::{
                block_device_apply, decide_ownership, get_udev_property, UdevOwnership,
                STRATIS_FS_TYPE,
            },
        },
        types::{ClevisInfo, DevUuid, DevicePath, EncryptionInfo, PoolUuid},
    },
    stratis::{StratisError, StratisResult},
};

const MIN_DEV_SIZE: Bytes = Bytes(IEC::Gi as u128);

lazy_static! {
    static ref BLOCKDEVS_IN_PROGRESS: Mutex<HashSet<PathBuf>> = Mutex::new(HashSet::new());
}

// Get information that can be obtained from udev for the block device
// identified by devnode. Return an error if there was an error finding the
// information or no udev entry corresponding to the devnode could be found.
// Return an error if udev ownership could not be obtained.
fn udev_info(
    devnode: &DevicePath,
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
            StratisError::Msg(format!(
                "Block device {} could not be found in the udev database",
                devnode.display()
            ))
        })
    })
    .map_err(|err| {
        StratisError::Msg(format!(
            "Could not obtain udev information for block device {}: {}",
            devnode.display(),
            err
        ))
    })
    .and_then(|(ownership, devnum, id_wwn)| {
        devnum
            .ok_or_else(|| {
                StratisError::Msg(format!(
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
                StratisError::Msg(format!(
                    "Could not obtain ownership information for device {} using udev: {}",
                    devnode.display(),
                    err
                ))
            })
    })
}

/// Get the device number of a device by stat-ing the device node.
fn get_devno_from_path(path: &Path) -> StratisResult<Device> {
    let info = stat(path)?;
    Ok(Device::from_kdev_t(convert_int!(info.st_rdev, u64, u32)?))
}

/// Find all devices that match the given pool and device UUIDs using libblkid.
///
/// This method is specifically a work around for cases where due to locking
/// internally in stratisd, udev events cannot be used for device identification
/// because they will not have the opportunity to be processed.
pub fn find_stratis_devs_by_uuid(
    pool_uuid: PoolUuid,
    uuids: Vec<DevUuid>,
) -> StratisResult<HashMap<DevUuid, (DevicePath, Device)>> {
    let mut map = HashMap::new();
    if uuids.is_empty() {
        return Ok(map);
    }

    let mut cache = BlkidCache::get_cache(None)?;
    cache.probe_all()?;
    for dev in cache.iter().search("TYPE", STRATIS_FS_TYPE)? {
        if let Some(dev) = cache.verify(dev) {
            let devname = DevicePath::new(&dev.devname()?)?;
            let dev_uuid = DevUuid::parse_str(cache.get_tag_value("UUID", &devname)?)?;
            let dev_pool_uuid = PoolUuid::parse_str(cache.get_tag_value("POOL_UUID", &devname)?)?;
            let devno = get_devno_from_path(&devname)?;

            if dev_pool_uuid == pool_uuid && uuids.contains(&dev_uuid) {
                info!(
                    "Found device with path: {}, pool UUID: {}, device UUID: {} after unlock",
                    devname.display(),
                    dev_pool_uuid,
                    dev_uuid
                );
                map.insert(dev_uuid, (DevicePath::new(&devname)?, devno));
            }
        }
    }
    cache.put_cache();
    Ok(map)
}

/// Verify that udev information using a blkid probe to search for superblocks
/// and number of partitions on the device.
///
/// Returns optional number of partitions and superblock type or error.
fn verify_device_with_blkid(path: &DevicePath) -> StratisResult<(Option<i32>, Option<String>)> {
    let mut probe = BlkidProbe::new_from_filename(path)?;
    probe.enable_superblocks(true)?;
    probe.enable_partitions(true)?;
    probe.do_safeprobe()?;

    let num_parts = probe
        .get_partitions()
        .and_then(|mut parts| parts.number_of_partitions())
        .ok();
    let superblock_type = probe.lookup_value("TYPE").ok();

    debug!(
        "Verifying device using blkid probe: superblock probe: {:?}, number of partitions: {:?}",
        superblock_type, num_parts
    );

    Ok((num_parts, superblock_type))
}

// Find information from the devnode that is useful to identify a device or
// to construct a StratBlockDev object. Returns a tuple of a DeviceInfo struct
// and Stratis identifiers for the device, if any are found. If the value for
// the Stratis identifiers is None, then this device has been determined to be
// unowned.
fn dev_info(devnode: &DevicePath) -> StratisResult<(DeviceInfo, Option<StratisIdentifiers>)> {
    let (ownership, devnum, hw_id) = udev_info(devnode)?;

    match ownership {
        UdevOwnership::Luks | UdevOwnership::MultipathMember | UdevOwnership::Theirs => {
            let err_str = format!(
                "udev information indicates that device {} is a {}",
                devnode.display(),
                ownership
            );
            Err(StratisError::Msg(err_str))
        }
        UdevOwnership::Stratis | UdevOwnership::Unowned => {
            let (num_parts, sublk_type) = verify_device_with_blkid(devnode)?;
            let (has_parts, sublk_is_stratis_or_unowned) = (
                num_parts.as_ref().map(|num| *num > 0).unwrap_or(false),
                sublk_type == Some("stratis".to_string()) || sublk_type.is_none(),
            );
            if !sublk_is_stratis_or_unowned || has_parts {
                return Err(StratisError::Msg(format!(
                    "Device {} was reported to be unowned by udev but actually contains existing partitions or superblock; partitions: {:?}, superblock: {:?}",
                    devnode.display(),
                    num_parts,
                    sublk_type,
                )));
            }

            let mut f = OpenOptions::new().read(true).write(true).open(&**devnode)?;
            let dev_size = blkdev_size(&f)?;

            let stratis_identifiers = device_identifiers(&mut f).map_err(|err| {
                let error_message = format!(
                    "There was an error reading Stratis metadata from device {}; the device is unsuitable for initialization: {}",
                    devnode.display(),
                    err
                );
                StratisError::Msg(error_message)
            })?;

            if ownership == UdevOwnership::Stratis && stratis_identifiers.is_none() {
                let error_message = format!(
                    "udev identified device {} as a Stratis device but device metadata does not show that it is a Stratis device",
                    devnode.display()
                );
                return Err(StratisError::Msg(error_message));
            }

            Ok((
                DeviceInfo {
                    devno: devnum,
                    devnode: devnode.to_path_buf(),
                    id_wwn: hw_id,
                    size: dev_size,
                },
                stratis_identifiers,
            ))
        }
    }
}

/// Check a device to determine whether it satisfies Stratis's device
/// requirements, return an error if not.
fn check_dev(device_info: &DeviceInfo) -> StratisResult<()> {
    if device_info.size < MIN_DEV_SIZE {
        let error_message = format!(
            "Device {} is {} which is smaller than the minimum required size for a Stratis blockdev, {}",
            device_info.devnode.display(),
            device_info.size,
            MIN_DEV_SIZE
        );
        Err(StratisError::Msg(error_message))
    } else {
        Ok(())
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

/// Devices that have all been identified as Stratis devices.
#[derive(Debug)]
pub struct StratisDevices {
    inner: HashMap<PoolUuid, HashMap<DevUuid, DeviceInfo>>,
}

impl StratisDevices {
    // Given a pool UUID partition the devices into two divisions;
    // those that belong to the pool and those that do not.
    pub fn partition(mut self, uuid: PoolUuid) -> (HashMap<DevUuid, DeviceInfo>, StratisDevices) {
        let this_pool = self.inner.remove(&uuid).unwrap_or_default();
        (this_pool, StratisDevices { inner: self.inner })
    }

    // Return an error message on the assumption that these devices have
    // been identified as belonging to another pool.
    pub fn error_on_not_empty(&self) -> StratisResult<()> {
        let errors = self
            .inner
            .iter()
            .map(|(pool_uuid, devs)| {
                format!(
                    "devices ({}) appear to belong to Stratis pool with UUID {}",
                    devs.iter()
                        .map(|(_, info)| info.devnode.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    pool_uuid
                )
            })
            .collect::<Vec<_>>();

        if errors.is_empty() {
            Ok(())
        } else {
            let error_message = format!(
                "Some devices specified appear to be already in use by other Stratis pools: {}",
                errors.join("; ")
            );
            Err(StratisError::Msg(error_message))
        }
    }
}

/// A list of device paths is converted into this structure.
/// Invariants:
/// * DeviceInfo devnode values are unique.
/// * DeviceInfo devno values are unique.
/// * DeviceInfo.size value meets the required Stratis minimum.
#[derive(Debug)]
pub struct ProcessedPathInfos {
    pub stratis_devices: HashMap<PoolUuid, HashMap<DevUuid, DeviceInfo>>,
    pub unclaimed_devices: Vec<DeviceInfo>,
}

impl ProcessedPathInfos {
    /// Unpack ProcessedPathInfos into devices owned by Stratis and
    /// into unowned devices.
    pub fn unpack(self) -> (StratisDevices, UnownedDevices) {
        (
            StratisDevices {
                inner: self.stratis_devices,
            },
            UnownedDevices {
                inner: self.unclaimed_devices,
            },
        )
    }
}

impl TryFrom<&[&Path]> for ProcessedPathInfos {
    type Error = StratisError;

    // Returns an error if information on any device can not be found
    // If paths is not empty then, either an error is returned OR at least one
    // of the fields of the result is not empty.
    fn try_from(paths: &[&Path]) -> StratisResult<Self> {
        let canonical_paths = paths
            .iter()
            .map(|p| {
                DevicePath::new(p).map_err(|err| {
                    StratisError::Chained(
                        format!(
                            "Unable to process specified device path \"{}\"",
                            p.display()
                        ),
                        Box::new(err),
                    )
                })
            })
            .collect::<StratisResult<Vec<DevicePath>>>()?;

        let mut infos = canonical_paths
            .iter()
            .unique()
            .map(|path| {
                dev_info(path).and_then(|(info, ids)| check_dev(&info).map(|_| (info, ids)))
            })
            .collect::<StratisResult<Vec<(DeviceInfo, Option<StratisIdentifiers>)>>>()
            .map_err(|err| {
                let error_message = format!(
                    "At least one of the devices specified was unsuitable for initialization: {}",
                    err
                );
                StratisError::Msg(error_message)
            })?;

        let duplicate_device_numbers = infos
            .iter()
            .duplicates_by(|(info, _)| info.devno)
            .collect::<Vec<_>>();

        let duplicate_device_number_messages = duplicate_device_numbers
            .iter()
            .map(|(info, _)| {
                let dups = infos
                    .iter()
                    .filter(|(i, _)| i.devno == info.devno)
                    .collect::<Vec<_>>();
                format!(
                    "device nodes {} correspond to device number {}",
                    dups.iter().map(|(i, _)| i.devnode.display()).join(", "),
                    info.devno
                )
            })
            .collect::<Vec<_>>();

        if !duplicate_device_number_messages.is_empty() {
            return Err(StratisError::Msg(
                duplicate_device_number_messages.join("; "),
            ));
        }

        let (mut stratis_devices, mut unclaimed_devices) = (HashMap::new(), vec![]);

        for (info, ids) in infos.drain(..) {
            match ids {
                Some(ids) => {
                    let entry = stratis_devices
                        .entry(ids.pool_uuid)
                        .or_insert_with(HashMap::new);
                    if let Some(old_info) = entry.insert(ids.device_uuid, info) {
                        return Err(StratisError::Msg(format!(
                            "Duplicate Stratis devices found: {} and {} both have the Stratis identifiers {}",
                            old_info.devnode.display(),
                            entry.get(&ids.device_uuid).expect("just inserted").devnode.display(),
                            ids
                        )));
                    }
                }
                None => unclaimed_devices.push(info),
            }
        }

        Ok(ProcessedPathInfos {
            stratis_devices,
            unclaimed_devices,
        })
    }
}

/// A list of device paths that have been determined to be unowned, and thus
/// can be initialized by stratisd.
/// Invariants:
/// * DeviceInfo devnode values are unique.
/// * DeviceInfo devno values are unique.
/// * DeviceInfo.size value meets the required Stratis minimum.
#[derive(Debug)]
pub struct UnownedDevices {
    pub(super) inner: Vec<DeviceInfo>,
}

impl UnownedDevices {
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn unpack(self) -> Vec<DeviceInfo> {
        self.inner
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
/// Precondition: Each device's DeviceInfo struct contains all necessary
/// information about the device.
pub fn initialize_devices(
    devices: UnownedDevices,
    pool_uuid: PoolUuid,
    mda_data_size: MDADataSize,
    encryption_info: Option<&EncryptionInfo>,
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
        key_description: Option<&KeyDescription>,
        enable_clevis: Option<&ClevisInfo>,
    ) -> StratisResult<(CryptHandle, Device, Sectors)> {
        let handle = CryptInitializer::new(DevicePath::new(physical_path)?, pool_uuid, dev_uuid)
            .initialize(key_description, enable_clevis)?;

        let device_size = match handle.logical_device_size() {
            Ok(size) => size,
            Err(error) => {
                let path = handle.luks2_device_path().display().to_string();
                if let Err(e) = handle.wipe() {
                    warn!(
                        "Failed to clean up encrypted device {}; cleanup \
                        was attempted because initialization of the device \
                        failed: {}",
                        path, e
                    );
                }
                return Err(error);
            }
        };
        map_device_nums(handle.activated_device_path()).map(|dn| (handle, dn, device_size))
    }

    fn initialize_stratis_metadata(
        underlying_device: UnderlyingDevice,
        devno: Device,
        pool_uuid: PoolUuid,
        dev_uuid: DevUuid,
        sizes: (MDADataSize, BlockdevSize),
        id_wwn: &Option<StratisResult<String>>,
    ) -> StratisResult<StratBlockDev> {
        let (mda_data_size, data_size) = sizes;
        let mut f = OpenOptions::new()
            .write(true)
            .open(underlying_device.metadata_path())?;

        // NOTE: Encrypted devices will discard the hardware ID as encrypted devices
        // are always represented as logical, software-based devicemapper devices
        // which will never have a hardware ID.
        let hw_id = match (underlying_device.crypt_handle().is_some(), id_wwn) {
            (true, _) => None,
            (_, Some(Ok(ref hw_id))) => Some(hw_id.to_owned()),
            (_, Some(Err(_))) => {
                warn!("Value for ID_WWN for device {} obtained from the udev database could not be decoded; inserting device into pool with UUID {} anyway",
                      underlying_device.physical_path().display(),
                      pool_uuid);
                None
            }
            (_, None) => None,
        };

        let bda = BDA::new(
            StratisIdentifiers::new(pool_uuid, dev_uuid),
            mda_data_size,
            data_size,
            Utc::now().timestamp() as u64,
        );

        bda.initialize(&mut f)?;

        StratBlockDev::new(devno, bda, &[], None, hw_id, underlying_device)
    }

    /// Clean up an encrypted device after initialization failure.
    fn clean_up_encrypted(handle: &mut CryptHandle, causal_error: StratisError) -> StratisError {
        if let Err(e) = handle.wipe() {
            let msg = format!(
                "Failed to clean up encrypted device {}; cleanup was attempted because initialization of the device failed",
                handle.luks2_device_path().display(),
            );
            warn!("{}; clean up failure cause: {}", msg, e,);
            StratisError::Chained(
                msg,
                Box::new(StratisError::NoActionRollbackError {
                    causal_error: Box::new(causal_error),
                    rollback_error: Box::new(e),
                }),
            )
        } else {
            causal_error
        }
    }

    /// Clean up an unencrypted device after initialization failure.
    fn clean_up_unencrypted(path: &Path, causal_error: StratisError) -> StratisError {
        if let Err(e) = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(StratisError::from)
            .and_then(|mut f| disown_device(&mut f))
        {
            let msg = format!(
                "Failed to clean up unencrypted device {}; cleanup was attempted because initialization of the device failed",
                path.display(),
            );
            warn!("{}; clean up failure cause: {}", msg, e,);
            StratisError::Chained(
                msg,
                Box::new(StratisError::NoActionRollbackError {
                    causal_error: Box::new(causal_error),
                    rollback_error: Box::new(e),
                }),
            )
        } else {
            causal_error
        }
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
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<StratBlockDev> {
        let dev_uuid = DevUuid::new_v4();
        let (handle, devno, blockdev_size) = if let Some(ei) = encryption_info {
            initialize_encrypted(
                &dev_info.devnode,
                pool_uuid,
                dev_uuid,
                ei.key_description(),
                ei.clevis_info(),
            )
            .map(|(handle, devno, devsize)| {
                debug!(
                    "Info on physical device {}, logical device {}",
                    &dev_info.devnode.display(),
                    handle.activated_device_path().display(),
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
                (Some(handle), devno, devsize)
            })?
        } else {
            (None, dev_info.devno, dev_info.size.sectors())
        };

        let physical_path = &dev_info.devnode;
        match handle {
            Some(handle) => {
                let mut handle_clone = handle.clone();
                let blockdev = initialize_stratis_metadata(
                    UnderlyingDevice::Encrypted(handle),
                    devno,
                    pool_uuid,
                    dev_uuid,
                    (mda_data_size, BlockdevSize::new(blockdev_size)),
                    &dev_info.id_wwn,
                );
                if let Err(err) = blockdev {
                    Err(clean_up_encrypted(&mut handle_clone, err))
                } else {
                    blockdev
                }
            }
            None => {
                let blockdev = initialize_stratis_metadata(
                    UnderlyingDevice::Unencrypted(DevicePath::new(physical_path)?),
                    devno,
                    pool_uuid,
                    dev_uuid,
                    (mda_data_size, BlockdevSize::new(blockdev_size)),
                    &dev_info.id_wwn,
                );
                if let Err(err) = blockdev {
                    Err(clean_up_unencrypted(physical_path, err))
                } else {
                    blockdev
                }
            }
        }
    }

    /// Initialize all provided devices with Stratis metadata.
    fn initialize_all(
        devices: UnownedDevices,
        pool_uuid: PoolUuid,
        mda_data_size: MDADataSize,
        encryption_info: Option<&EncryptionInfo>,
    ) -> StratisResult<Vec<StratBlockDev>> {
        let mut initialized_blockdevs: Vec<StratBlockDev> = Vec::new();
        for dev_info in devices.inner {
            match initialize_one(&dev_info, pool_uuid, mda_data_size, encryption_info) {
                Ok(blockdev) => initialized_blockdevs.push(blockdev),
                Err(err) => {
                    if let Err(err) = wipe_blockdevs(&mut initialized_blockdevs) {
                        warn!("Failed to clean up some devices after initialization of device {} for pool with UUID {} failed: {}",
                              dev_info.devnode.display(),
                              pool_uuid,
                              err);
                    }
                    return Err(err);
                }
            }
        }
        Ok(initialized_blockdevs)
    }

    let device_paths = devices
        .inner
        .iter()
        .map(|d| d.devnode.clone())
        .collect::<Vec<_>>();
    {
        let mut guard = (*BLOCKDEVS_IN_PROGRESS).lock().expect("Should not panic");
        if device_paths.iter().any(|dev| guard.contains(dev)) {
            return Err(StratisError::Msg(format!("An initialization operation is already in progress with at least one of the following devices: {:?}", device_paths)));
        }
        guard.extend(device_paths.iter().cloned());
    }

    let res = initialize_all(devices, pool_uuid, mda_data_size, encryption_info);

    {
        let mut guard = (*BLOCKDEVS_IN_PROGRESS).lock().expect("Should not panic");
        guard.retain(|path| !device_paths.contains(path));
    }

    res
}

/// Wipe some blockdevs of their identifying headers.
/// Return an error if any of the blockdevs could not be wiped.
/// If an error occurs while wiping a blockdev, attempt to wipe all remaining.
pub fn wipe_blockdevs(blockdevs: &mut [StratBlockDev]) -> StratisResult<()> {
    let unerased_devnodes: Vec<_> = blockdevs
        .iter_mut()
        .filter_map(|bd| match bd.disown() {
            Err(e) => Some((bd.physical_path(), e)),
            _ => None,
        })
        .collect();

    if unerased_devnodes.is_empty() {
        Ok(())
    } else {
        let errors =
            unerased_devnodes
                .into_iter()
                .fold(Vec::new(), |mut errs, (devnode, next_error)| {
                    errs.push(StratisError::Chained(
                        format!("Failed to wipe block device {}", devnode.display(),),
                        Box::new(next_error),
                    ));
                    errs
                });
        let err_msg = "Failed to wipe already initialized devnodes".to_string();
        Err(StratisError::BestEffortError(err_msg, errors))
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs::OpenOptions};

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
        let pool_uuid = PoolUuid::new_v4();
        let dev_infos: Vec<_> = ProcessedPathInfos::try_from(paths)?.unclaimed_devices;

        if dev_infos.len() != paths.len() {
            return Err(Box::new(StratisError::Msg(
                "Some duplicate devices were found".to_string(),
            )));
        }

        let mut blockdevs = initialize_devices(
            UnownedDevices { inner: dev_infos },
            pool_uuid,
            MDADataSize::default(),
            key_description
                .map(|kd| EncryptionInfo::KeyDesc(kd.clone()))
                .as_ref(),
        )?;

        if blockdevs.len() != paths.len() {
            return Err(Box::new(StratisError::Msg(
                "Fewer blockdevices were created than were requested".to_string(),
            )));
        }

        let stratis_devnodes: Vec<PathBuf> = blockdevs
            .iter()
            .map(|bd| bd.metadata_path().to_owned())
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
            return Err(Box::new(StratisError::Msg(
                "Some device which should have had Stratis identifiers on it did not".to_string(),
            )));
        }

        if stratis_identifiers
            .iter()
            .any(|x| x.expect("returned in line above if any are None").pool_uuid != pool_uuid)
        {
            return Err(Box::new(StratisError::Msg(
                "Some device had the wrong pool UUID".to_string(),
            )));
        }

        if key_description.is_none() {
            if !ProcessedPathInfos::try_from(
                stratis_devnodes
                    .iter()
                    .map(|p| p.as_path())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap()
            .unpack()
            .1
            .inner
            .is_empty()
            {
                return Err(Box::new(StratisError::Msg(
                    "Failed to eliminate devices already initialized for this pool from list of devices to initialize".to_string()
                )));
            }

            if ProcessedPathInfos::try_from(
                stratis_devnodes
                    .iter()
                    .map(|p| p.as_path())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap()
            .unpack()
            .0
            .partition(pool_uuid)
            .1
            .error_on_not_empty()
            .is_err()
            {
                return Err(Box::new(StratisError::Msg(
                    "Failed to return an error when some device processed was not in the set of already initialized devices".to_string()
                )));
            }
        } else {
            // The devices will be rejected with an errorif they were the
            // minimum size when initialized.
            if let Ok(infos) = ProcessedPathInfos::try_from(
                stratis_devnodes
                    .iter()
                    .map(|p| p.as_path())
                    .collect::<Vec<_>>()
                    .as_slice(),
            ) {
                if !infos.unpack().0.partition(pool_uuid).1.inner.is_empty() {
                    return Err(Box::new(StratisError::Msg(
                        "Failed to eliminate devices already initialized for this pool from list of devices to initialize".to_string()
                    )));
                }
            }

            if ProcessedPathInfos::try_from(paths).is_ok() {
                return Err(Box::new(StratisError::Msg(
                    "Failed to return an error when encountering devices that are LUKS2"
                        .to_string(),
                )));
            }
        }

        if let Ok(infos) = ProcessedPathInfos::try_from(
            stratis_devnodes
                .iter()
                .map(|p| p.as_path())
                .collect::<Vec<_>>()
                .as_slice(),
        ) {
            if !infos.unpack().0.partition(PoolUuid::new_v4()).0.is_empty() {
                return Err(Box::new(StratisError::Msg(
                    "Failed to leave devices in StratisDevices when processing devices for a pool UUID which is not the same as that for which the devices were initialized".to_string()
                )));
            }
        };

        wipe_blockdevs(&mut blockdevs)?;

        for path in paths {
            if key_description.is_some() {
                if CryptHandle::setup(path)?.is_some() {
                    return Err(Box::new(StratisError::Msg(
                        "LUKS2 metadata on Stratis devices was not successfully wiped".to_string(),
                    )));
                }
            } else if device_identifiers(&mut OpenOptions::new().read(true).open(path)?)? != None {
                return Err(Box::new(StratisError::Msg(
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

    // Verify that a non-existent path results in a reasonably elegant
    // error, i.e., not an assertion failure.
    fn test_nonexistent_path(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let test_paths = [paths, &[Path::new("/srk/cheese")]].concat();

        assert_matches!(ProcessedPathInfos::try_from(test_paths.as_slice()), Err(_));
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

    // Verify that if the last device in a list of devices to initialize
    // can not be initialized, all the devices previously initialized are
    // properly cleaned up.
    fn test_failure_cleanup(
        paths: &[&Path],
        key_desc: Option<&KeyDescription>,
    ) -> Result<(), Box<dyn Error>> {
        if paths.len() <= 1 {
            return Err(Box::new(StratisError::Msg(
                "Test requires more than one device".to_string(),
            )));
        }

        let mut dev_infos = ProcessedPathInfos::try_from(paths)?.unclaimed_devices;
        let pool_uuid = PoolUuid::new_v4();

        if dev_infos.len() != paths.len() {
            return Err(Box::new(StratisError::Msg(
                "Some duplicate devices were found".to_string(),
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

        if initialize_devices(
            UnownedDevices { inner: dev_infos },
            pool_uuid,
            MDADataSize::default(),
            key_desc
                .map(|kd| EncryptionInfo::KeyDesc(kd.clone()))
                .as_ref(),
        )
        .is_ok()
        {
            return Err(Box::new(StratisError::Msg(
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
                    return Err(Box::new(StratisError::Msg(format!(
                        "Device {} should have no LUKS2 metadata",
                        path.display()
                    ))));
                }
            } else {
                let mut f = OpenOptions::new().read(true).write(true).open(path)?;
                match device_identifiers(&mut f) {
                    Ok(None) => (),
                    _ => {
                        return Err(Box::new(StratisError::Msg(format!(
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

    // Verify that resolve devices simply eliminates duplicate devnodes,
    // without returning an error.
    fn test_duplicate_devnodes(paths: &[&Path]) {
        assert!(!paths.is_empty());

        let duplicate_paths = paths
            .iter()
            .chain(paths.iter())
            .copied()
            .collect::<Vec<_>>();

        let result = ProcessedPathInfos::try_from(duplicate_paths.as_slice())
            .unwrap()
            .unclaimed_devices;

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
}
