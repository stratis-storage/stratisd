// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ffi::CString,
    io,
    path::{Path, PathBuf},
};

use libc::{c_void, syscall, SYS_keyctl, SYS_request_key};

use devicemapper::Sectors;
use libcryptsetup_rs::{
    c_uint, CryptActivateFlags, CryptDeactivateFlags, CryptDevice, CryptInit, CryptStatusInfo,
    CryptVolumeKeyFlags, CryptWipePattern, EncryptionFormat, LibcryptErr, SafeMemHandle,
};

use crate::engine::{DevUuid, PoolUuid};

type Result<T> = std::result::Result<T, LibcryptErr>;

// Stratis token JSON keys
const TOKEN_TYPE_KEY: &str = "type";
const TOKEN_KEYSLOTS_KEY: &str = "keyslots";
const STRATIS_TOKEN_DEVNAME_KEY: &str = "activation_name";
const STRATIS_TOKEN_KEYDESC_KEY: &str = "key_description";
const STRATIS_TOKEN_POOL_UUID_KEY: &str = "pool_uuid";
const STRATIS_TOKEN_DEV_UUID_KEY: &str = "device_uuid";

const LUKS2_TOKEN_ID: c_uint = 0;
const STRATIS_TOKEN_ID: c_uint = 1;

const LUKS2_TOKEN_TYPE: &str = "luks2-keyring";
const STRATIS_TOKEN_TYPE: &str = "stratis";
#[cfg(not(test))]
const STRATIS_MEK_SIZE: usize = 512 / 8;
#[cfg(test)]
pub const STRATIS_MEK_SIZE: usize = 512 / 8;

/// Sector size as determined in `cryptsetup/lib/internal.h`
const SECTOR_SIZE: u64 = 512;

/// Path to logical devices for encrypted devices
const DEVICEMAPPER_PATH: &str = "/dev/mapper";

macro_rules! log_on_failure {
    ($op:expr, $fmt:tt $(, $arg:expr)*) => {{
        let result = $op;
        if let Err(ref e) = result {
            warn!(
                concat!($fmt, "; failed with error: {}"),
                $($arg,)*
                e
            );
        }
        result?
    }}
}

/// Get a devicemapper name from the pool and device UUIDs.
pub fn name_from_uuids(pool_uuid: &PoolUuid, dev_uuid: &DevUuid) -> String {
    format!("{}-{}", pool_uuid.to_simple_ref(), dev_uuid.to_simple_ref())
}

/// Handle for initialization actions on a physical device.
pub struct CryptInitializer {
    physical_path: PathBuf,
    pool_uuid: PoolUuid,
    dev_uuid: DevUuid,
}

impl CryptInitializer {
    pub fn new(physical_path: PathBuf, pool_uuid: PoolUuid, dev_uuid: DevUuid) -> Self {
        CryptInitializer {
            physical_path,
            pool_uuid,
            dev_uuid,
        }
    }

    pub fn initialize(self, key_description: &str) -> Result<CryptHandle> {
        let physical_path = self.physical_path.clone();
        let pool_uuid = self.pool_uuid;
        let dev_uuid = self.dev_uuid;
        let device = log_on_failure!(
            CryptInit::init(physical_path.as_path()),
            "Failed to acquire context for device {} while initializing; \
            nothing to clean up",
            physical_path.display()
        );
        let result = self.initialize_no_cleanup(device, key_description);
        result.map_err(|device| {
            if let Err(e) = CryptInitializer::rollback(
                device,
                physical_path,
                name_from_uuids(&pool_uuid, &dev_uuid),
            ) {
                e
            } else {
                LibcryptErr::Other("Device initialization failed".to_string())
            }
        })
    }

    fn initialize_with_err(&self, device: &mut CryptDevice, key_description: &str) -> Result<()> {
        log_on_failure!(
            device.context_handle().format::<()>(
                EncryptionFormat::Luks2,
                ("aes", "xts-plain64"),
                None,
                libcryptsetup_rs::Either::Right(STRATIS_MEK_SIZE),
                None,
            ),
            "Failed to format device {} with LUKS2 header",
            self.physical_path.display()
        );
        let key = log_on_failure!(
            read_key(key_description),
            "Failed to read key with key description {} from keyring",
            key_description
        );

        let keyslot = log_on_failure!(
            device.keyslot_handle().add_by_key(
                None,
                None,
                #[cfg(cryptsetup_compat)]
                (*key).deref(),
                #[cfg(not(cryptsetup_compat))]
                key.as_ref(),
                CryptVolumeKeyFlags::empty(),
            ),
            "Failed to initialize keyslot with provided key in keyring"
        );

        // Initialize keyring token
        log_on_failure!(
            device
                .token_handle()
                .luks2_keyring_set(Some(LUKS2_TOKEN_ID), key_description),
            "Failed to initialize the LUKS2 token for driving keyring activation operations"
        );
        log_on_failure!(
            device
                .token_handle()
                .assign_keyslot(LUKS2_TOKEN_ID, Some(keyslot)),
            "Failed to assign the LUKS2 keyring token to the Stratis keyslot"
        );

        // The default activation name is [POOLUUID]-[DEVUUID] which should be unique
        // across all Stratis pools.
        let activation_name = name_from_uuids(&self.pool_uuid, &self.dev_uuid);

        // Initialize stratis token
        log_on_failure!(
            device.token_handle().json_set(
                Some(STRATIS_TOKEN_ID),
                &json!({
                    TOKEN_TYPE_KEY: STRATIS_TOKEN_TYPE,
                    TOKEN_KEYSLOTS_KEY: [],
                    STRATIS_TOKEN_POOL_UUID_KEY: self.pool_uuid.to_simple_ref().to_string(),
                    STRATIS_TOKEN_DEV_UUID_KEY: self.dev_uuid.to_simple_ref().to_string(),
                    STRATIS_TOKEN_KEYDESC_KEY: key_description,
                    STRATIS_TOKEN_DEVNAME_KEY: activation_name,
                }),
            ),
            "Failed to create the Stratis token"
        );

        activate_and_check_device_path(device, &activation_name)
    }

    /// Lay down properly configured LUKS2 metadata on a new physical device
    fn initialize_no_cleanup(
        self,
        mut device: CryptDevice,
        key_description: &str,
    ) -> std::result::Result<CryptHandle, CryptDevice> {
        let pool_uuid = self.pool_uuid;
        let dev_uuid = self.dev_uuid;
        let result = self.initialize_with_err(&mut device, key_description);
        match result {
            Ok(_) => Ok(CryptHandle::new(
                device,
                self.physical_path,
                name_from_uuids(&pool_uuid, &dev_uuid),
            )),
            Err(e) => {
                warn!("Initialization failed with error: {}; rolling back.", e);
                Err(device)
            }
        }
    }

    pub fn rollback(mut device: CryptDevice, physical_path: PathBuf, name: String) -> Result<()> {
        ensure_wiped(&mut device, &physical_path, &name)
    }
}

/// Handle for crypt device operations on an existing crypt device loaded
/// from a physical device.
pub struct CryptHandle {
    device: CryptDevice,
    physical_path: PathBuf,
    name: String,
}

impl CryptHandle {
    /// Create new
    pub(crate) fn new(device: CryptDevice, physical_path: PathBuf, name: String) -> Self {
        CryptHandle {
            device,
            physical_path,
            name,
        }
    }

    #[cfg(test)]
    pub fn can_setup(physical_path: &Path) -> bool {
        fn can_setup_with_failures(physical_path: &Path) -> Result<bool> {
            let mut device = log_on_failure!(
                CryptInit::init(physical_path),
                "Failed to acquire a context for device {}",
                physical_path.display()
            );
            if device
                .context_handle()
                .load::<()>(EncryptionFormat::Luks2, None)
                .is_err()
            {
                Ok(false)
            } else {
                Ok(is_encrypted_stratis_device(&mut device))
            }
        }

        can_setup_with_failures(physical_path)
            .map_err(|e| {
                warn!(
                    "Failed to check if device {} is a compatible encrypted Stratis \
                    device: {}; Reporting as not a Stratis device.",
                    physical_path.display(),
                    e
                );
            })
            .unwrap_or(false)
    }

    pub fn setup(physical_path: &Path) -> Result<Option<Self>> {
        let mut device = log_on_failure!(
            CryptInit::init(physical_path),
            "Failed to acquire handle to device {} while loading from disk",
            physical_path.display()
        );
        if device
            .context_handle()
            .load::<()>(EncryptionFormat::Luks2, None)
            .is_err()
            || !is_encrypted_stratis_device(&mut device)
        {
            return Ok(None);
        }
        let name = CryptHandle::name_from_metadata(&mut device)?;
        Ok(Some(CryptHandle {
            device,
            physical_path: physical_path.to_owned(),
            name,
        }))
    }

    /// Return the path to the device node of the underlying physical device
    /// for the encrypted device.
    pub fn physical_device_path(&self) -> &Path {
        self.physical_path.as_path()
    }

    /// Get the logical path to use for unencrypted operations that is mapped
    /// to and stored on the encrypted physical device.
    ///
    /// * Returns `Some` with the logical path if the device node generated
    ///   from the name exists.
    /// * Returns `None` if the logical path expected based on the activation name
    ///   of the devicemapper device does not exist.
    pub fn logical_device_path(&self) -> Option<PathBuf> {
        let mut logical_path = PathBuf::from(DEVICEMAPPER_PATH);
        logical_path.push(self.name.as_str());
        if logical_path.exists() {
            Some(logical_path)
        } else {
            None
        }
    }

    /// Query the Stratis metadata for the device activation name.
    fn name_from_metadata(device: &mut CryptDevice) -> Result<String> {
        let json = log_on_failure!(
            device.token_handle().json_get(STRATIS_TOKEN_ID),
            "Failed to get Stratis JSON token from LUKS2 metadata"
        );
        let name = log_on_failure!(
            json.get(STRATIS_TOKEN_DEVNAME_KEY)
                .and_then(|type_val| type_val.as_str())
                .map(|type_str| type_str.to_string())
                .ok_or_else(|| {
                    LibcryptErr::Other(
                        "Malformed or missing JSON value for activation_name".to_string(),
                    )
                }),
            "Could not get value for key activation_name from Stratis JSON token"
        );
        Ok(name)
    }

    /// Activate encrypted Stratis device using the name stored in the
    /// Stratis token
    #[allow(dead_code)]
    pub fn activate(&mut self) -> Result<()> {
        activate_and_check_device_path(&mut self.device, &self.name.to_owned())
    }

    /// Deactivate the device referenced by the current device handle.
    #[allow(dead_code)]
    pub fn deactivate(&mut self) -> Result<()> {
        let name = self.name.to_owned();
        ensure_inactive(&mut self.device, &name)
    }

    /// Wipe all LUKS2 metadata on the device safely using libcryptsetup.
    pub fn wipe(&mut self) -> Result<()> {
        let physical_path = self.physical_path.to_owned();
        let name = self.name.to_owned();
        ensure_wiped(&mut self.device, &physical_path, &name)
    }

    /// Get the size of the logical device built on the underlying encrypted physical
    /// device. `devicemapper` will return the size in terms of number of sectors.
    pub fn logical_device_size(&mut self) -> Result<Sectors> {
        let name = self.name.clone();
        let active_device = log_on_failure!(
            self.device.runtime_handle(&name).get_active_device(),
            "Failed to get device size for encrypted logical device"
        );
        Ok(Sectors(active_device.size))
    }
}

/// Check whether the physical device path corresponds to an encrypted
/// Stratis device.
///
/// This method works on activated and deactivated encrypted devices.
///
/// This device will only return true if the device was initialized
/// with encryption by Stratis. This requires that:
/// * the device is a LUKS2 encrypted device.
/// * the device has a valid Stratis LUKS2 token.
fn is_encrypted_stratis_device(device: &mut CryptDevice) -> bool {
    fn device_operations(device: &mut CryptDevice) -> Result<bool> {
        check_luks2_token(device)?;

        // Checking the LUKS2 token type not be entirely necessary as we've already
        // validated that the token can decrypt the volume which should also
        // validate that the appropriate key is in the keyring.
        //
        // Only potential benefit of checking the type of the token is that a LUKS2 token
        // of another type may be present and capable of unlocking a non-Stratis device
        // using something other than the keyring for example.
        //
        // Given that only Stratis devices should have a Stratis token, this may
        // still be superfluous as we will check for that after before determining
        // whether or not it is a Stratis device and nothing is ever activated so
        // we do not need to guard against unlocking a device that is not ours.
        let luks_json = log_on_failure!(
            device.token_handle().json_get(LUKS2_TOKEN_ID),
            "Failed to get LUKS2 keyring JSON token"
        );
        let stratis_json = log_on_failure!(
            device.token_handle().json_get(STRATIS_TOKEN_ID),
            "Failed to get Stratis JSON token"
        );
        if !luks2_token_type_is_valid(&luks_json) || !stratis_token_is_valid(&stratis_json) {
            Ok(false)
        } else {
            Ok(true)
        }
    }

    device_operations(device)
        .map_err(|e| {
            warn!(
                "Operations querying device to determine if it is a Stratis device \
            failed with an error: {}; reporting as not a Stratis device.",
                e
            );
        })
        .unwrap_or(false)
}

fn device_is_active(device: &mut CryptDevice, device_name: &str) -> bool {
    libcryptsetup_rs::status(Some(device), device_name)
        .map(|status| status == CryptStatusInfo::Active)
        .unwrap_or(false)
}

/// Activate device by token then check that the logical path exists corresponding
/// to the activation name passed into this method.
fn activate_and_check_device_path(crypt_device: &mut CryptDevice, name: &str) -> Result<()> {
    // Activate by token
    log_on_failure!(
        crypt_device.token_handle().activate_by_token::<()>(
            Some(name),
            Some(LUKS2_TOKEN_ID),
            None,
            CryptActivateFlags::empty(),
        ),
        "Failed to activate device with name {}",
        name
    );

    // Check activation status.
    if !device_is_active(crypt_device, name) {
        warn!(
            "Activation reported success but device does not appear to be \
            active"
        );
        return Err(LibcryptErr::Other("Device activation failed".to_string()));
    }

    // Checking that the symlink was created may also be valuable in case a race
    // condition occurs with udev.
    let mut activated_path = PathBuf::from(DEVICEMAPPER_PATH);
    activated_path.push(name);

    // Can potentially use inotify with a timeout to wait for the symlink
    // if race conditions become a problem.
    if activated_path.exists() {
        Ok(())
    } else {
        Err(LibcryptErr::IOError(io::Error::from(
            io::ErrorKind::NotFound,
        )))
    }
}

/// Get a list of all keyslots associated with the LUKS2 token.
/// This is necessary because attempting to destroy an uninitialized
/// keyslot will result in an error.
fn get_keyslot_number(device: &mut CryptDevice) -> Result<Vec<c_uint>> {
    let json = log_on_failure!(
        device.token_handle().json_get(LUKS2_TOKEN_ID),
        "Failed to get the JSON LUKS2 keyring token from the assigned keyslot"
    );
    let vec = json
        .get(TOKEN_KEYSLOTS_KEY)
        .and_then(|k| k.as_array())
        .ok_or_else(|| LibcryptErr::Other("keyslots value was malformed".to_string()))?;
    Ok(vec
        .iter()
        .filter_map(|int_val| {
            let as_str = int_val.as_str();
            if as_str.is_none() {
                warn!(
                    "Discarding invalid value in LUKS2 token keyslot array: {}",
                    int_val
                );
            }
            let s = match as_str {
                Some(s) => s,
                None => return None,
            };
            let as_u64 = s.parse::<u64>();
            if let Err(ref e) = as_u64 {
                warn!(
                    "Discarding invalid value in LUKS2 token keyslot array: {}; \
                    failed to convert it to an integer: {}",
                    s, e,
                );
            }
            as_u64.ok()
        })
        .map(|int| int as c_uint)
        .collect::<Vec<_>>())
}

/// Deactivate an encrypted Stratis device but do not wipe it. This is not
/// a destructive action. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup. This method is idempotent and leaves
/// the state as inactive.
fn ensure_inactive(device: &mut CryptDevice, name: &str) -> Result<()> {
    if log_on_failure!(
        libcryptsetup_rs::status(Some(device), name),
        "Failed to determine status of device with name {}",
        name
    ) == CryptStatusInfo::Active
    {
        log_on_failure!(
            device
                .activate_handle()
                .deactivate(name, CryptDeactivateFlags::empty()),
            "Failed to deactivate the crypt device with name {}",
            name
        );
    }
    if log_on_failure!(
        libcryptsetup_rs::status(Some(device), name),
        "Failed to determine status of device with name {}",
        name
    ) != CryptStatusInfo::Inactive
    {
        warn!(
            "Device deactivation of name {} reported success but device \
            still reports something other than an inactive status",
            name
        );
        return Err(LibcryptErr::Other(
            "Deactivation of device failed.".to_string(),
        ));
    }
    Ok(())
}

/// Align the number of bytes to the nearest multiple of `SECTOR_SIZE`
/// above the current value.
fn ceiling_sector_size_alignment(bytes: u64) -> u64 {
    bytes + (SECTOR_SIZE - (bytes % SECTOR_SIZE))
}

/// Deactivate an encrypted Stratis device and wipe it. This is
/// a destructive action and data will be unrecoverable from this device
/// after this operation. `name` should be the name of the device as registered
/// with devicemapper and cryptsetup. `physical_path` should be the path to
/// the device node of the physical storage backing the encrypted volume.
/// This method is idempotent and leaves the disk as wiped.
fn ensure_wiped(device: &mut CryptDevice, physical_path: &Path, name: &str) -> Result<()> {
    ensure_inactive(device, name)?;
    let keyslot_number = get_keyslot_number(device);
    match keyslot_number {
        Ok(nums) => {
            for i in nums.iter() {
                log_on_failure!(
                    device.keyslot_handle().destroy(*i),
                    "Failed to destroy keyslot at index {}",
                    i
                );
            }
        }
        Err(e) => {
            info!(
                "Keyslot numbers were not found; skipping explicit \
                destruction of keyslots; the keyslot area will still \
                be wiped in the next step: {}",
                e,
            );
        }
    }

    let (md_size, ks_size) = log_on_failure!(
        device.settings_handle().get_metadata_size(),
        "Failed to acquire LUKS2 metadata size"
    );
    debug!("Metadata size of LUKS2 device: {}", *md_size);
    debug!("Keyslot area size of LUKS2 device: {}", *ks_size);
    let total_luks2_metadata_size = ceiling_sector_size_alignment(*md_size * 2 + *ks_size);
    debug!("Aligned total size: {}", total_luks2_metadata_size);

    log_on_failure!(
        device.wipe_handle().wipe::<()>(
            physical_path,
            CryptWipePattern::Zero,
            0,
            total_luks2_metadata_size,
            SECTOR_SIZE as usize,
            false,
            None,
            None,
        ),
        "Failed to wipe device with name {}",
        name
    );
    Ok(())
}

/// Check that the token can open the device.
///
/// No activation will actually occur, only validation.
fn check_luks2_token(device: &mut CryptDevice) -> Result<()> {
    log_on_failure!(
        device.token_handle().activate_by_token::<()>(
            None,
            Some(LUKS2_TOKEN_ID),
            None,
            CryptActivateFlags::empty(),
        ),
        "libcryptsetup reported that the LUKS2 token is unable to \
        open the encrypted device; this could be due to a malformed \
        LUKS2 keyring token on the device or a missing or inaccessible \
        key in the keyring"
    );
    Ok(())
}

/// Validate that the LUKS2 token is present and valid
///
/// May not be necessary. See the comment above the invocation.
fn luks2_token_type_is_valid(json: &serde_json::Value) -> bool {
    json.get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == LUKS2_TOKEN_TYPE)
        .unwrap_or(false)
}

/// Validate that the Stratis token is present and valid
fn stratis_token_is_valid(json: &serde_json::Value) -> bool {
    debug!("Stratis LUKS2 token: {}", json);

    let type_valid = json
        .get(TOKEN_TYPE_KEY)
        .and_then(|type_val| type_val.as_str())
        .map(|type_str| type_str == STRATIS_TOKEN_TYPE)
        .unwrap_or(false);
    if !type_valid {
        debug!("Type value is invalid");
    }

    let keyslots_valid = json
        .get(TOKEN_KEYSLOTS_KEY)
        .and_then(|arr| arr.as_array())
        .map(|arr| arr.is_empty())
        .unwrap_or(false);
    if !keyslots_valid {
        debug!("Keyslots value is invalid");
    }

    let key_desc_valid = json
        .get(STRATIS_TOKEN_KEYDESC_KEY)
        .map(|key| key.is_string())
        .unwrap_or(false);
    if !key_desc_valid {
        debug!("Key description value is invalid");
    }

    let pool_uuid_valid = json
        .get(STRATIS_TOKEN_POOL_UUID_KEY)
        .and_then(|uuid| uuid.as_str())
        .and_then(|uuid_str| uuid::Uuid::parse_str(uuid_str).ok())
        .is_some();
    if !pool_uuid_valid {
        debug!("Pool UUID value is invalid");
    }

    let dev_uuid_valid = json
        .get(STRATIS_TOKEN_DEV_UUID_KEY)
        .and_then(|uuid| uuid.as_str())
        .and_then(|uuid_str| uuid::Uuid::parse_str(uuid_str).ok())
        .is_some();
    if !dev_uuid_valid {
        debug!("Device UUID value is invalid");
    }

    let device_name_valid = json.get(STRATIS_TOKEN_DEVNAME_KEY).is_some();
    if !device_name_valid {
        debug!("Device name value is invalid");
    }

    type_valid
        && keyslots_valid
        && key_desc_valid
        && pool_uuid_valid
        && dev_uuid_valid
        && device_name_valid
}

/// Read key from keyring with the given key description
///
/// Returns a safe owned memory segment that will clear itself when dropped.
///
/// Requires cryptsetup 2.3
fn read_key(key_description: &str) -> Result<SafeMemHandle> {
    fn read_key_result(key_description: &str) -> Result<SafeMemHandle> {
        // Attach persistent keyring to process keyring
        match unsafe {
            syscall(
                SYS_keyctl,
                libc::KEYCTL_GET_PERSISTENT,
                0,
                libc::KEY_SPEC_SESSION_KEYRING,
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            _ => (),
        };

        let key_type_cstring = CString::new("user").expect("String is valid");
        let key_description_cstring =
            CString::new(key_description).map_err(LibcryptErr::NullError)?;

        // Request key ID from persistent keyring
        let key_id = match unsafe {
            syscall(
                SYS_request_key,
                key_type_cstring.as_ptr(),
                key_description_cstring.as_ptr(),
                std::ptr::null::<c_void>(),
                0,
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            i => i,
        };

        let mut key_buffer = SafeMemHandle::alloc(STRATIS_MEK_SIZE)?;
        let mut_ref = key_buffer.as_mut();

        // Read key from keyring
        match unsafe {
            syscall(
                SYS_keyctl,
                libc::KEYCTL_READ,
                key_id,
                mut_ref.as_mut_ptr(),
                mut_ref.len(),
            )
        } {
            i if i < 0 => return Err(LibcryptErr::IOError(io::Error::last_os_error())),
            _ => (),
        };
        Ok(key_buffer)
    }

    let read_key_result = read_key_result(key_description);
    if read_key_result.is_err() {
        warn!(
            "Failed to read the key with key description {} from the keyring; \
            encryption cannot continue",
            key_description
        );
    }
    read_key_result
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{File, OpenOptions},
        io::{Read, Write},
    };

    use uuid::Uuid;

    use devicemapper::Sectors;

    use crate::{
        engine::strat_engine::tests::{crypt, loopbacked, real},
        stratis::StratisError,
    };

    use super::*;

    /// If this method is called without a key with the specified key description
    /// in the kernel ring, it should always fail and allow us to test the rollback
    /// of failed initializations.
    fn test_failed_init(paths: &[&Path]) {
        assert_eq!(paths.len(), 1);

        let path = paths.get(0).expect("There must be exactly one path");
        let key_description = "I am not a key";

        let pool_uuid = Uuid::new_v4();
        let dev_uuid = Uuid::new_v4();

        let result = CryptInitializer::new((*path).to_owned(), pool_uuid, dev_uuid)
            .initialize(key_description);

        // Initialization cannot occur with a non-existent key
        assert!(result.is_err());

        assert!(!CryptHandle::can_setup(path));

        // TODO: Check actual superblock with libblkid
    }

    #[test]
    fn loop_test_failed_init() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_failed_init,
        );
    }

    #[test]
    fn real_test_failed_init() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_failed_init,
        );
    }

    #[test]
    fn travis_test_failed_init() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_failed_init,
        );
    }

    /// Test initializing and activating an encrypted device using
    /// the utilities provided here.
    fn test_crypt_device_ops(paths: &[&Path]) {
        fn crypt_test(paths: &[&Path], key_desc: &str) -> std::result::Result<(), Box<dyn Error>> {
            let path = paths.get(0).ok_or_else(|| {
                Box::new(StratisError::Error(
                    "This test only accepts a single device".to_string(),
                ))
            })?;

            let pool_uuid = Uuid::new_v4();
            let dev_uuid = Uuid::new_v4();

            let mut handle = CryptInitializer::new((*path).to_owned(), pool_uuid, dev_uuid)
                .initialize(key_desc)?;
            let logical_path = handle.logical_device_path().ok_or_else(|| {
                Box::new(StratisError::Error(
                    "Logical path does not exist".to_string(),
                ))
            })?;

            let mut devicenode = OpenOptions::new().write(true).open(logical_path)?;
            let mut random_buffer = [0; 32];
            File::open("/dev/urandom")?.read_exact(&mut random_buffer)?;
            devicenode.write_all(&random_buffer)?;
            std::mem::drop(devicenode);

            let mut disk_buffer = Vec::new();
            let mut devicenode = File::open(path)?;
            devicenode.read_to_end(&mut disk_buffer)?;
            for window in disk_buffer.windows(32) {
                if window == random_buffer {
                    return Err(Box::new(io::Error::new(
                        io::ErrorKind::Other,
                        "Disk was not encrypted!",
                    )));
                }
            }
            std::mem::drop(devicenode);

            handle.deactivate()?;

            handle.activate()?;
            handle.wipe()?;

            Ok(())
        }

        assert_eq!(paths.len(), 1);

        crypt::insert_and_cleanup_key(paths, crypt_test);
    }

    #[test]
    fn loop_test_crypt_device_ops() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn real_test_crypt_device_ops() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn travis_test_crypt_device_ops() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_crypt_device_ops,
        );
    }
}
