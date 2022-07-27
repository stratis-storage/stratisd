// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
mod macros;

mod activate;
mod consts;
mod handle;
mod initialize;
mod metadata_handle;
mod shared;

pub use self::{
    activate::CryptActivationHandle,
    consts::CLEVIS_TANG_TRUST_URL,
    handle::CryptHandle,
    initialize::CryptInitializer,
    metadata_handle::CryptMetadataHandle,
    shared::{
        back_up_luks_header, crypt_metadata_size, interpret_clevis_config, restore_luks_header,
    },
};

#[cfg(test)]
mod tests {
    use std::{
        convert::TryFrom,
        env,
        error::Error,
        ffi::CString,
        fs::{File, OpenOptions},
        io::{self, Read, Write},
        mem::MaybeUninit,
        path::Path,
        ptr, slice,
    };

    use devicemapper::Sectors;
    use libcryptsetup_rs::{CryptInit, CryptStatusInfo, Either, EncryptionFormat};

    use crate::{
        engine::{
            strat_engine::{
                backstore::crypt::{
                    consts::{
                        CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE,
                        DEFAULT_CRYPT_METADATA_SIZE, LUKS2_TOKEN_ID, STRATIS_MEK_SIZE,
                    },
                    shared::acquire_crypt_device,
                },
                ns::{unshare_namespace, MemoryFilesystem},
                tests::{crypt, loopbacked, real},
            },
            types::{DevUuid, DevicePath, KeyDescription, PoolUuid, UnlockMethod},
        },
        stratis::StratisError,
    };

    use super::*;

    /// If this method is called without a key with the specified key description
    /// in the kernel ring, it should always fail and allow us to test the rollback
    /// of failed initializations.
    fn test_failed_init(paths: &[&Path]) {
        assert_eq!(paths.len(), 1);

        let path = paths.get(0).expect("There must be exactly one path");
        let key_description =
            KeyDescription::try_from("I am not a key".to_string()).expect("no semi-colons");

        let pool_uuid = PoolUuid::new_v4();
        let dev_uuid = DevUuid::new_v4();

        let result = CryptInitializer::new(DevicePath::new(path).unwrap(), pool_uuid, dev_uuid)
            .initialize(Some(&key_description), None);

        // Initialization cannot occur with a non-existent key
        assert!(result.is_err());

        assert!(CryptHandle::setup(path).unwrap().is_none());

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

    /// Test the method `can_unlock` works on an initialized device in both
    /// active and inactive states.
    fn test_can_unlock(paths: &[&Path]) {
        fn crypt_test(
            paths: &[&Path],
            key_desc: &KeyDescription,
        ) -> std::result::Result<(), Box<dyn Error>> {
            let mut handles = vec![];

            let pool_uuid = PoolUuid::new_v4();
            for path in paths {
                let dev_uuid = DevUuid::new_v4();

                let handle = CryptInitializer::new(DevicePath::new(path)?, pool_uuid, dev_uuid)
                    .initialize(Some(key_desc), None)?;
                handles.push(handle);
            }

            for path in paths {
                if !CryptActivationHandle::can_unlock(path, true, false) {
                    return Err(Box::new(StratisError::Msg(
                        "All devices should be able to be unlocked".to_string(),
                    )));
                }
            }

            for handle in handles.iter_mut() {
                handle.deactivate()?;
            }

            for path in paths {
                if !CryptActivationHandle::can_unlock(path, true, false) {
                    return Err(Box::new(StratisError::Msg(
                        "All devices should be able to be unlocked".to_string(),
                    )));
                }
            }

            for handle in handles.iter_mut() {
                handle.wipe()?;
            }

            for path in paths {
                if CryptActivationHandle::can_unlock(path, true, false) {
                    return Err(Box::new(StratisError::Msg(
                        "All devices should no longer be able to be unlocked".to_string(),
                    )));
                }
            }

            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, crypt_test)
    }

    #[test]
    fn loop_test_can_unlock() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Range(1, 3, None),
            test_can_unlock,
        );
    }

    #[test]
    fn real_test_can_unlock() {
        real::test_with_spec(
            &real::DeviceLimits::Range(1, 3, None, None),
            test_can_unlock,
        );
    }

    /// Test initializing and activating an encrypted device using
    /// the utilities provided here.
    ///
    /// The overall format of the test involves generating a random byte buffer
    /// of size 1 MiB, encrypting it on disk, and then ensuring that the plaintext
    /// cannot be found on the encrypted disk by doing a scan of the disk using
    /// a sliding window.
    ///
    /// The sliding window size of 1 MiB was chosen to lower the number of
    /// searches that need to be done compared to a smaller sliding window
    /// and also to decrease the probability of the random sequence being found
    /// on the disk due to leftover data from other tests.
    // TODO: Rewrite libc calls using nix crate.
    fn test_crypt_device_ops(paths: &[&Path]) {
        fn crypt_test(
            paths: &[&Path],
            key_desc: &KeyDescription,
        ) -> std::result::Result<(), Box<dyn Error>> {
            let path = paths.get(0).ok_or_else(|| {
                Box::new(StratisError::Msg(
                    "This test only accepts a single device".to_string(),
                ))
            })?;

            let pool_uuid = PoolUuid::new_v4();
            let dev_uuid = DevUuid::new_v4();

            let handle = CryptInitializer::new(DevicePath::new(path)?, pool_uuid, dev_uuid)
                .initialize(Some(key_desc), None)?;
            let logical_path = handle.activated_device_path();

            const WINDOW_SIZE: usize = 1024 * 1024;
            let mut devicenode = OpenOptions::new().write(true).open(logical_path)?;
            let mut random_buffer = Box::new([0; WINDOW_SIZE]);
            File::open("/dev/urandom")?.read_exact(&mut *random_buffer)?;
            devicenode.write_all(&*random_buffer)?;
            std::mem::drop(devicenode);

            let dev_path_cstring = CString::new(path.to_str().ok_or_else(|| {
                Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to convert path to string",
                ))
            })?)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            let fd = unsafe { libc::open(dev_path_cstring.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                return Err(Box::new(io::Error::last_os_error()));
            }

            let mut stat: MaybeUninit<libc::stat> = MaybeUninit::zeroed();
            let fstat_result = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
            if fstat_result < 0 {
                return Err(Box::new(io::Error::last_os_error()));
            }
            let device_size =
                convert_int!(unsafe { stat.assume_init() }.st_size, libc::off_t, usize)?;
            let mapped_ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    device_size,
                    libc::PROT_READ,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if mapped_ptr.is_null() {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::Other,
                    "mmap failed",
                )));
            }

            {
                let disk_buffer =
                    unsafe { slice::from_raw_parts(mapped_ptr as *const u8, device_size) };
                for window in disk_buffer.windows(WINDOW_SIZE) {
                    if window == &*random_buffer as &[u8] {
                        unsafe {
                            libc::munmap(mapped_ptr, device_size);
                            libc::close(fd);
                        };
                        return Err(Box::new(io::Error::new(
                            io::ErrorKind::Other,
                            "Disk was not encrypted!",
                        )));
                    }
                }
            }

            unsafe {
                libc::munmap(mapped_ptr, device_size);
                libc::close(fd);
            };

            let device_name = handle.name().to_owned();
            loop {
                match libcryptsetup_rs::status(
                    Some(&mut handle.acquire_crypt_device().unwrap()),
                    &device_name,
                ) {
                    Ok(CryptStatusInfo::Busy) => (),
                    Ok(CryptStatusInfo::Active) => break,
                    Ok(s) => {
                        return Err(Box::new(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Crypt device is in invalid state {:?}", s),
                        )))
                    }
                    Err(e) => {
                        return Err(Box::new(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Checking device status returned error: {}", e),
                        )))
                    }
                }
            }

            handle.deactivate()?;

            let handle =
                CryptActivationHandle::setup(path, UnlockMethod::Keyring)?.ok_or_else(|| {
                    Box::new(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "Device {} no longer appears to be a LUKS2 device",
                            path.display(),
                        ),
                    ))
                })?;
            handle.wipe()?;

            Ok(())
        }

        assert_eq!(paths.len(), 1);

        crypt::insert_and_cleanup_key(paths, crypt_test);
    }

    #[test]
    fn real_test_crypt_device_ops() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_crypt_device_ops,
        );
    }

    #[test]
    fn loop_test_crypt_metadata_defaults() {
        fn test_defaults(paths: &[&Path]) {
            let mut context = CryptInit::init(paths[0]).unwrap();
            context
                .context_handle()
                .format::<()>(
                    EncryptionFormat::Luks2,
                    ("aes", "xts-plain64"),
                    None,
                    Either::Right(STRATIS_MEK_SIZE),
                    None,
                )
                .unwrap();
            let (metadata, keyslot) = context.settings_handle().get_metadata_size().unwrap();
            assert_eq!(DEFAULT_CRYPT_METADATA_SIZE, *metadata);
            assert_eq!(DEFAULT_CRYPT_KEYSLOTS_SIZE, *keyslot);
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), test_defaults);
    }

    fn test_both_initialize(paths: &[&Path]) {
        fn both_initialize(
            paths: &[&Path],
            key_desc: &KeyDescription,
        ) -> Result<(), Box<dyn Error>> {
            unshare_namespace()?;
            let _memfs = MemoryFilesystem::new()?;
            let path = paths
                .get(0)
                .copied()
                .ok_or_else(|| StratisError::Msg("Expected exactly one path".to_string()))?;
            let handle = CryptInitializer::new(
                DevicePath::new(path)?,
                PoolUuid::new_v4(),
                DevUuid::new_v4(),
            )
            .initialize(
                Some(key_desc),
                Some(&(
                    "tang".to_string(),
                    json!({"url": env::var("TANG_URL")?, "stratis:tang:trust_url": true}),
                )),
            )?;

            let mut device = acquire_crypt_device(handle.luks2_device_path())?;
            device.token_handle().json_get(LUKS2_TOKEN_ID)?;
            device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID)?;
            Ok(())
        }

        crypt::insert_and_cleanup_key(paths, both_initialize);
    }

    #[test]
    fn clevis_real_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_both_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_both_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_both_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_both_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_both_initialize,
        );
    }

    fn test_clevis_initialize(paths: &[&Path]) {
        unshare_namespace().unwrap();
        let _memfs = MemoryFilesystem::new().unwrap();
        let path = paths[0];
        let handle = CryptInitializer::new(
            DevicePath::new(path).unwrap(),
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
        )
        .initialize(
            None,
            Some(&(
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").unwrap(), "stratis:tang:trust_url": true}),
            )),
        )
        .unwrap();

        let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
        assert!(device.token_handle().json_get(CLEVIS_LUKS_TOKEN_ID).is_ok());
        assert!(device.token_handle().json_get(LUKS2_TOKEN_ID).is_err());
    }

    #[test]
    fn clevis_real_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_real_should_fail_test_initialize() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(1024 * 1024 * 1024 / 512))),
            test_clevis_initialize,
        );
    }

    #[test]
    fn clevis_loop_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_initialize,
        );
    }

    #[test]
    #[should_panic]
    fn clevis_loop_should_fail_test_initialize() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_initialize,
        );
    }
}
