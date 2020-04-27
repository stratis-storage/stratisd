// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, fs::File, io::Read, path::Path};

use libcryptsetup_rs::SafeMemHandle;

use crate::engine::{
    engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
    strat_engine::keys::StratKeyActions,
    types::SizedKeyMemory,
};

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
    let mut key_handle = StratKeyActions;
    let desc_str = "test-description-for-stratisd";
    let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
    File::open("/dev/urandom")
        .unwrap()
        .read_exact(mem.as_mut())
        .unwrap();
    let key_data = SizedKeyMemory::new(mem, MAX_STRATIS_PASS_SIZE);

    key_handle.add_no_fd(desc_str, key_data)?;

    let result = test(physical_paths, desc_str, input);

    key_handle.delete(desc_str)?;

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
