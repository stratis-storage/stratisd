// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
mod macros;

mod consts;
mod handle;
mod shared;

pub use self::{
    consts::CLEVIS_TANG_TRUST_URL,
    handle::CryptHandle,
    shared::{
        back_up_luks_header, crypt_metadata_size, register_clevis_token, restore_luks_header,
        set_up_crypt_logging,
    },
};

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::CString,
        fs::{File, OpenOptions},
        io::{self, Read, Write},
        mem::MaybeUninit,
        path::Path,
        ptr, slice,
    };

    use devicemapper::{Bytes, Sectors, IEC};
    use libcryptsetup_rs::{
        consts::vals::{CryptStatusInfo, EncryptionFormat},
        CryptInit, Either,
    };

    use crate::engine::{
        strat_engine::{
            backstore::crypt::{
                consts::{
                    CLEVIS_LUKS_TOKEN_ID, DEFAULT_CRYPT_KEYSLOTS_SIZE, DEFAULT_CRYPT_METADATA_SIZE,
                    LUKS2_TOKEN_ID, STRATIS_MEK_SIZE,
                },
                shared::acquire_crypt_device,
            },
            ns::{unshare_mount_namespace, MemoryFilesystem},
            tests::{crypt, loopbacked, real},
        },
        types::{DevUuid, EncryptionInfo, KeyDescription, Name, PoolUuid, UnlockMethod},
    };

    use super::*;

    /// If this method is called without a key with the specified key description
    /// in the kernel ring, it should always fail and allow us to test the rollback
    /// of failed initializations.
    fn test_failed_init(paths: &[&Path]) {
        assert_eq!(paths.len(), 1);

        let path = paths.first().expect("There must be exactly one path");
        let key_description =
            KeyDescription::try_from("I am not a key".to_string()).expect("no semi-colons");

        let pool_uuid = PoolUuid::new_v4();
        let pool_name = Name::new("pool_name".to_string());
        let dev_uuid = DevUuid::new_v4();

        let result = CryptHandle::initialize(
            path,
            pool_uuid,
            dev_uuid,
            pool_name,
            &EncryptionInfo::KeyDesc(key_description),
            None,
        );

        // Initialization cannot occur with a non-existent key
        assert!(result.is_err());

        assert!(CryptHandle::load_metadata(path).unwrap().is_none());

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
        fn crypt_test(paths: &[&Path], key_desc: &KeyDescription) {
            let mut handles = vec![];

            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            for path in paths {
                let dev_uuid = DevUuid::new_v4();

                let handle = CryptHandle::initialize(
                    path,
                    pool_uuid,
                    dev_uuid,
                    pool_name.clone(),
                    &EncryptionInfo::KeyDesc(key_desc.clone()),
                    None,
                )
                .unwrap();
                handles.push(handle);
            }

            for path in paths {
                if !CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should be able to be unlocked");
                }
            }

            for handle in handles.iter_mut() {
                handle.deactivate().unwrap();
            }

            for path in paths {
                if !CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should be able to be unlocked");
                }
            }

            for handle in handles.iter_mut() {
                handle.wipe().unwrap();
            }

            for path in paths {
                if CryptHandle::can_unlock(path, true, false) {
                    panic!("All devices should no longer be able to be unlocked");
                }
            }
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
        fn crypt_test(paths: &[&Path], key_desc: &KeyDescription) {
            let path = paths
                .first()
                .expect("This test only accepts a single device");

            let pool_uuid = PoolUuid::new_v4();
            let pool_name = Name::new("pool_name".to_string());
            let dev_uuid = DevUuid::new_v4();

            let handle = CryptHandle::initialize(
                path,
                pool_uuid,
                dev_uuid,
                pool_name,
                &EncryptionInfo::KeyDesc(key_desc.clone()),
                None,
            )
            .unwrap();
            let logical_path = handle.activated_device_path();

            const WINDOW_SIZE: usize = 1024 * 1024;
            let mut devicenode = OpenOptions::new().write(true).open(logical_path).unwrap();
            let mut random_buffer = vec![0; WINDOW_SIZE].into_boxed_slice();
            File::open("/dev/urandom")
                .unwrap()
                .read_exact(&mut random_buffer)
                .unwrap();
            devicenode.write_all(&random_buffer).unwrap();
            std::mem::drop(devicenode);

            let dev_path_cstring =
                CString::new(path.to_str().expect("Failed to convert path to string")).unwrap();
            let fd = unsafe { libc::open(dev_path_cstring.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                panic!("{:?}", io::Error::last_os_error());
            }

            let mut stat: MaybeUninit<libc::stat> = MaybeUninit::zeroed();
            let fstat_result = unsafe { libc::fstat(fd, stat.as_mut_ptr()) };
            if fstat_result < 0 {
                panic!("{:?}", io::Error::last_os_error());
            }
            let device_size =
                convert_int!(unsafe { stat.assume_init() }.st_size, libc::off_t, usize).unwrap();
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
                panic!("mmap failed");
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
                        panic!("Disk was not encrypted!");
                    }
                }
            }

            unsafe {
                libc::munmap(mapped_ptr, device_size);
                libc::close(fd);
            };

            let device_name = handle.activation_name();
            loop {
                match libcryptsetup_rs::status(
                    Some(&mut handle.acquire_crypt_device().unwrap()),
                    &device_name.to_string(),
                ) {
                    Ok(CryptStatusInfo::Busy) => (),
                    Ok(CryptStatusInfo::Active) => break,
                    Ok(s) => {
                        panic!("Crypt device is in invalid state {s:?}")
                    }
                    Err(e) => {
                        panic!("Checking device status returned error: {e}")
                    }
                }
            }

            handle.deactivate().unwrap();

            let handle = CryptHandle::setup(path, Some(UnlockMethod::Keyring))
                .unwrap()
                .unwrap_or_else(|| {
                    panic!(
                        "Device {} no longer appears to be a LUKS2 device",
                        path.display(),
                    )
                });
            handle.wipe().unwrap();
        }

        assert_eq!(paths.len(), 1);

        crypt::insert_and_cleanup_key(paths, crypt_test);
    }

    #[test]
    fn real_test_crypt_device_ops() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, Some(Sectors(2 * IEC::Mi))),
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
            assert_eq!(DEFAULT_CRYPT_METADATA_SIZE, Bytes::from(*metadata));
            assert_eq!(DEFAULT_CRYPT_KEYSLOTS_SIZE, Bytes::from(*keyslot));
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), test_defaults);
    }

    #[test]
    // Test passing an unusual, larger sector size for cryptsetup. 4096 should
    // be no smaller than the physical sector size of the loop device, and
    // should be allowed by cryptsetup.
    fn loop_test_set_sector_size() {
        fn the_test(paths: &[&Path]) {
            fn test_set_sector_size(paths: &[&Path], key_description: &KeyDescription) {
                let pool_uuid = PoolUuid::new_v4();
                let pool_name = Name::new("pool_name".to_string());
                let dev_uuid = DevUuid::new_v4();

                CryptHandle::initialize(
                    paths[0],
                    pool_uuid,
                    dev_uuid,
                    pool_name,
                    &EncryptionInfo::KeyDesc(key_description.clone()),
                    Some(4096u32),
                )
                .unwrap();
            }

            crypt::insert_and_cleanup_key(paths, test_set_sector_size);
        }

        loopbacked::test_with_spec(&loopbacked::DeviceLimits::Exactly(1, None), the_test);
    }

    fn test_both_initialize(paths: &[&Path]) {
        fn both_initialize(paths: &[&Path], key_desc: &KeyDescription) {
            unshare_mount_namespace().unwrap();
            let _memfs = MemoryFilesystem::new().unwrap();
            let path = paths.first().copied().expect("Expected exactly one path");
            let pool_name = Name::new("pool_name".to_string());
            let handle = CryptHandle::initialize(
                path,
                PoolUuid::new_v4(),
                DevUuid::new_v4(),
                pool_name,
                &EncryptionInfo::Both(
                    key_desc.clone(),
                    (
                        "tang".to_string(),
                        json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
                    ),
                ),
                None,
            ).unwrap();

            let mut device = acquire_crypt_device(handle.luks2_device_path()).unwrap();
            device.token_handle().json_get(LUKS2_TOKEN_ID).unwrap();
            device
                .token_handle()
                .json_get(CLEVIS_LUKS_TOKEN_ID)
                .unwrap();
            handle.deactivate().unwrap();
        }

        fn unlock_clevis(paths: &[&Path]) {
            let path = paths.first().copied().expect("Expected exactly one path");
            CryptHandle::setup(path, Some(UnlockMethod::Clevis))
                .unwrap()
                .unwrap();
        }

        crypt::insert_and_remove_key(paths, both_initialize, unlock_clevis);
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
        unshare_mount_namespace().unwrap();

        let _memfs = MemoryFilesystem::new().unwrap();
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        let handle = CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required"), "stratis:tang:trust_url": true}),
            )),
            None,
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

    fn test_clevis_tang_configs(paths: &[&Path]) {
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name.clone(),
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({"url": env::var("TANG_URL").expect("TANG_URL env var required")}),
            )),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            &EncryptionInfo::ClevisInfo((
                "tang".to_string(),
                json!({
                    "stratis:tang:trust_url": true,
                    "url": env::var("TANG_URL").expect("TANG_URL env var required"),
                }),
            )),
            None,
        )
        .unwrap();
    }

    #[test]
    fn clevis_real_test_clevis_tang_configs() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_clevis_tang_configs,
        );
    }

    #[test]
    fn clevis_loop_test_clevis_tang_configs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_tang_configs,
        );
    }

    fn test_clevis_sss_configs(paths: &[&Path]) {
        let path = paths[0];
        let pool_name = Name::new("pool_name".to_string());

        assert!(CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name.clone(),
            &EncryptionInfo::ClevisInfo((
                "sss".to_string(),
                json!({"t": 1, "pins": {"tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")}, "tpm2": {}}}),
            )),
            None,
        )
        .is_err());
        CryptHandle::initialize(
            path,
            PoolUuid::new_v4(),
            DevUuid::new_v4(),
            pool_name,
            &EncryptionInfo::ClevisInfo((
                "sss".to_string(),
                json!({
                    "t": 1,
                    "stratis:tang:trust_url": true,
                    "pins": {
                        "tang": {"url": env::var("TANG_URL").expect("TANG_URL env var required")},
                        "tpm2": {}
                    }
                }),
            )),
            None,
        )
        .unwrap();
    }

    #[test]
    fn clevis_real_test_clevis_sss_configs() {
        real::test_with_spec(
            &real::DeviceLimits::Exactly(1, None, None),
            test_clevis_sss_configs,
        );
    }

    #[test]
    fn clevis_loop_test_clevis_sss_configs() {
        loopbacked::test_with_spec(
            &loopbacked::DeviceLimits::Exactly(1, None),
            test_clevis_sss_configs,
        );
    }
}
