// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, error::Error, fs::File, io::Read, path::Path};

use libcryptsetup_rs::SafeMemHandle;

use crate::engine::{
    engine::{KeyActions, MAX_STRATIS_PASS_SIZE},
    strat_engine::{keys::StratKeyActions, names::KeyDescription},
    types::SizedKeyMemory,
};

/// Generate a random key and associate it with the given key description.
fn generate_random_key(key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
    let mut mem = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;
    File::open("/dev/urandom")?.read_exact(mem.as_mut())?;
    let key_data = SizedKeyMemory::new(mem, MAX_STRATIS_PASS_SIZE);

    StratKeyActions::set_no_fd(key_desc, key_data)?;
    Ok(())
}

/// Set up a key in the kernel keyring and return the key description.
fn set_up_key(desc_str: &str) -> KeyDescription {
    let key_description = KeyDescription::try_from(desc_str.to_string()).expect("no semi-colons");

    generate_random_key(&key_description).unwrap();

    key_description
}

/// Takes physical device paths from loopback or real tests and passes
/// them through to a compatible test definition. This method
/// will also enrich the context passed to the test with a key description
/// pointing to a key in the kernel keyring that has been randomly generated
/// and added for this test. It will always be cleaned up after the test completes
/// on both success and failure.
pub fn insert_and_cleanup_key<F>(physical_paths: &[&Path], test: F)
where
    F: Fn(&[&Path], &KeyDescription) -> std::result::Result<(), Box<dyn Error>>,
{
    let key_description = set_up_key("test-description-for-stratisd");

    let result = test(physical_paths, &key_description);

    StratKeyActions.unset(&key_description).unwrap();

    result.unwrap()
}

/// Takes physical device paths from loopback or real tests and passes
/// them through to a compatible test definition. This method
/// will also enrich the context passed to the test with two different key
/// descriptions pointing to keys in the kernel keyring that have been randomly
/// generated and added for this test. They will always be cleaned up after the
/// test completes on both success and failure.
pub fn insert_and_cleanup_two_keys<F>(physical_paths: &[&Path], test: F)
where
    F: Fn(&[&Path], &KeyDescription, &KeyDescription) -> std::result::Result<(), Box<dyn Error>>,
{
    let key_description1 = set_up_key("test-description-for-stratisd-1");
    let key_description2 = set_up_key("test-description-for-stratisd-2");

    let result = test(physical_paths, &key_description1, &key_description2);

    StratKeyActions.unset(&key_description1).unwrap();
    StratKeyActions.unset(&key_description2).unwrap();

    result.unwrap()
}

/// Keep the key description the same but change the data to a different key
/// to test that stratisd can appropriately handle such a case without getting
/// into a bad state.
pub fn change_key(key_desc: &KeyDescription) -> Result<(), Box<dyn Error>> {
    generate_random_key(key_desc)
}
