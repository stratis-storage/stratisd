// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, ffi::CString, fs::File, io::Read, path::Path};

use crate::engine::strat_engine::backstore::STRATIS_KEY_SIZE;

use self::consts::*;

/// Will be replaced with libc constants in libc v0.2.68
mod consts {
    use libc::c_int;

    pub const KEY_SPEC_SESSION_KEYRING: c_int = -3;
}

pub fn insert_and_cleanup_key<F>(physical_paths: &[&Path], test: F)
where
    F: Fn(&[&Path], &str) -> std::result::Result<(), Box<dyn Error>>,
{
    let type_cstring = "user\0";
    let description = "test-description-for-stratisd";
    let description_cstring = CString::new(description).unwrap();
    let mut key_data = [0; STRATIS_KEY_SIZE];
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(&mut key_data)
        .unwrap();

    // This constant is not in the libc crate yet
    const KEYCTL_UNLINK: i32 = 9;

    let key_id = match unsafe {
        libc::syscall(
            libc::SYS_add_key,
            type_cstring.as_ptr(),
            description_cstring.as_ptr(),
            key_data.as_ptr(),
            key_data.len(),
            KEY_SPEC_SESSION_KEYRING,
        )
    } {
        i if i < 0 => panic!("Failed to create key in keyring"),
        i => i,
    };

    let result = test(physical_paths, description);

    if unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_UNLINK,
            key_id,
            KEY_SPEC_SESSION_KEYRING,
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
