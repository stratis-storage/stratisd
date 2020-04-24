// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, ffi::CString, fs::File, io::Read, path::Path};

use crate::engine::{engine::MAX_STRATIS_PASS_SIZE, strat_engine::names::KeyDescription};

/// Takes physical device paths from loopback or real tests and passes
/// them through to a compatible test definition. This method
/// will also enrich the context passed to the test with a key description
/// pointing to a key in the kernel keyring that has been randomly generated
/// and added for this test. It will always be cleaned up after the test completes
/// on both success and failure.
fn insert_and_cleanup_key_shared<F, I, O>(
    physical_paths: &[&Path],
    test: F,
    input: I,
) -> Result<O, Box<dyn Error>>
where
    F: Fn(&[&Path], &str, I) -> std::result::Result<O, Box<dyn Error>>,
{
    let type_cstring = "user\0";
    let desc_str = "test-description-for-stratisd";
    let description = KeyDescription::from(desc_str.to_string());
    let description_cstring = CString::new(description.to_string()).unwrap();
    let mut key_data = [0; MAX_STRATIS_PASS_SIZE];
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

    let result = test(physical_paths, desc_str, input);

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

    result
}

/// Insert and clean up a single key for the lifetime of the test.
pub fn insert_and_cleanup_key<F>(physical_paths: &[&Path], test: F)
where
    F: Fn(&[&Path], &str, Option<()>) -> std::result::Result<(), Box<dyn Error>>,
{
    insert_and_cleanup_key_shared::<F, Option<()>, ()>(physical_paths, test, Option::<()>::None)
        .unwrap();
}

/// Keep the key description the same but change the data to a different key
/// to test that stratisd can appropriately handle such a case without getting
/// into a bad state.
pub fn insert_and_cleanup_two_keys<FR, F, R>(physical_paths: &[&Path], test_one: FR, test_two: F)
where
    FR: Fn(&[&Path], &str, Option<()>) -> Result<R, Box<dyn Error>>,
    F: Fn(&[&Path], &str, R) -> Result<(), Box<dyn Error>>,
{
    let return_value =
        insert_and_cleanup_key_shared::<FR, Option<()>, R>(physical_paths, test_one, None).unwrap();
    insert_and_cleanup_key_shared::<F, R, ()>(physical_paths, test_two, return_value).unwrap();
}
