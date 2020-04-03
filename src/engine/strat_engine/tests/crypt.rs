// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, ffi::CString, fs::File, io::Read, path::Path};

use crate::engine::strat_engine::backstore::STRATIS_PASS_SIZE;

/// Takes physical device paths from loopback or real tests and passes
/// them through to a compatible test definition. This method
/// will also enrich the context passed to the test with a key description
/// pointing to a key in the kernel keyring that has been randomly generated
/// and added for this test. It will always be cleaned up after the test completes
/// on both success and failure.
pub fn insert_and_cleanup_key<F>(physical_paths: &[&Path], test: F)
where
    F: Fn(&[&Path], &str) -> std::result::Result<(), Box<dyn Error>>,
{
    let type_cstring = "user\0";
    let description = "test-description-for-stratisd";
    let description_cstring = CString::new(description).unwrap();
    let mut key_data = [0; STRATIS_PASS_SIZE];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut key_data)
        .unwrap();

    let key_id = match unsafe {
        libc::syscall(
            libc::SYS_add_key,
            type_cstring.as_ptr(),
            description_cstring.as_ptr(),
            key_data.as_ptr(),
            key_data.len(),
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    } {
        i if i < 0 => panic!("Failed to create key in keyring"),
        i => i,
    };

    let result = test(physical_paths, description);

    if unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            libc::KEYCTL_UNLINK,
            key_id,
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    } < 0
    {
        panic!(
            "Failed to clean up key with key description {} from keyring",
            description
        );
    }

    result.unwrap()
}
