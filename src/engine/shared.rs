// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashSet},
    fs::File,
    io::Read,
    iter::FromIterator,
    os::unix::io::{AsRawFd, FromRawFd, RawFd},
    path::{Path, PathBuf},
};

use nix::poll::{poll, PollFd, PollFlags};
use regex::Regex;

use devicemapper::Bytes;
use libcryptsetup_rs::SafeMemHandle;

use crate::{
    engine::{
        engine::{Pool, MAX_STRATIS_PASS_SIZE},
        types::{BlockDevTier, CreateAction, DevUuid, PoolUuid, SetCreateAction, SizedKeyMemory},
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Called when the name of a requested pool coincides with the name of an
/// existing pool. Returns an error if the specifications of the requested
/// pool differ from the specifications of the existing pool, otherwise
/// returns Ok(CreateAction::Identity).
pub fn create_pool_idempotent_or_err(
    pool: &dyn Pool,
    pool_name: &str,
    blockdev_paths: &[&Path],
) -> StratisResult<CreateAction<PoolUuid>> {
    let input_devices: HashSet<PathBuf, RandomState> =
        blockdev_paths.iter().map(|p| p.to_path_buf()).collect();

    let existing_paths: HashSet<PathBuf, _> = pool
        .blockdevs()
        .iter()
        .filter_map(|(_, tier, bd)| {
            if *tier == BlockDevTier::Data {
                Some(bd.devnode().physical_path().to_owned())
            } else {
                None
            }
        })
        .collect();

    if input_devices == existing_paths {
        Ok(CreateAction::Identity)
    } else {
        let in_input = input_devices
            .difference(&existing_paths)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let in_pool = existing_paths
            .difference(&input_devices)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        Err(StratisError::Engine(
            ErrorEnum::Invalid,
            create_pool_generate_error_string!(pool_name, in_input, in_pool),
        ))
    }
}

/// Called when the name of a requested pool coincides with the name of an
/// existing pool. Returns an error if the specifications of the requested
/// pool differ from the specifications of the existing pool, otherwise
/// returns Ok(CreateAction::Identity).
pub fn init_cache_idempotent_or_err<I>(
    blockdev_paths: &[&Path],
    existing_iter: I,
) -> StratisResult<SetCreateAction<DevUuid>>
where
    I: Iterator<Item = PathBuf>,
{
    let input_devices = HashSet::from_iter(blockdev_paths.iter().map(|p| p.to_path_buf()));
    let existing_devices = HashSet::<_, RandomState>::from_iter(existing_iter);
    if input_devices == existing_devices {
        Ok(SetCreateAction::empty())
    } else {
        let in_input = input_devices
            .difference(&existing_devices)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let in_pool = existing_devices
            .difference(&input_devices)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        Err(StratisError::Engine(
            ErrorEnum::Invalid,
            init_cache_generate_error_string!(in_input, in_pool),
        ))
    }
}

/// Shared implementation of setting keys in the keyring for both the strat_engine
/// and sim_engine.
pub fn set_key_shared(key_fd: RawFd) -> StratisResult<SizedKeyMemory> {
    let mut key_file = unsafe { File::from_raw_fd(key_fd) };
    let mut memory = SafeMemHandle::alloc(MAX_STRATIS_PASS_SIZE)?;

    let bytes_read = key_file.read(memory.as_mut())?;

    if bytes_read == MAX_STRATIS_PASS_SIZE {
        let mut pollers = [PollFd::new(key_file.as_raw_fd(), PollFlags::POLLIN)];
        let num_events = poll(&mut pollers, 0)?;
        if num_events > 0 {
            return Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!(
                    "Provided key exceeded maximum allow length of {}",
                    Bytes(MAX_STRATIS_PASS_SIZE as u128)
                ),
            ));
        }
    }

    let sized_memory = SizedKeyMemory::new(memory, bytes_read);

    Ok(sized_memory)
}

/// Validate a str for use as a Pool or Filesystem name.
pub fn validate_name(name: &str) -> StratisResult<()> {
    if name.contains('\u{0}') {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name contains NULL characters : {}", name),
        ));
    }
    if name == "." || name == ".." {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name is . or .. : {}", name),
        ));
    }
    // Linux has a maximum filename length of 255 bytes
    if name.len() > 255 {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name has more than 255 bytes : {}", name),
        ));
    }
    if name.len() != name.trim().len() {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name contains leading or trailing space : {}", name),
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name contains control characters : {}", name),
        ));
    }
    lazy_static! {
        static ref NAME_UDEVREGEX: Regex =
            Regex::new(r"[[:ascii:]&&[^0-9A-Za-z#+-.:=@_/]]+").expect("regex is valid");
    }
    if NAME_UDEVREGEX.is_match(name) {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "Name contains characters not allowed in udev symlinks : {}",
                name
            ),
        ));
    }

    let name_path = Path::new(name);
    if name_path.components().count() != 1 {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name is a path with 0 or more than 1 components : {}", name),
        ));
    }
    if name_path.is_absolute() {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name is an absolute path : {}", name),
        ));
    }
    Ok(())
}

/// Verify that all paths are absolute.
pub fn validate_paths(paths: &[&Path]) -> StratisResult<()> {
    let non_absolute_paths: Vec<&Path> = paths
        .iter()
        .filter(|path| !path.is_absolute())
        .cloned()
        .collect();
    if non_absolute_paths.is_empty() {
        Ok(())
    } else {
        Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!(
                "Paths{{{}}} are not absolute",
                non_absolute_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_validate_name() {
        assert_matches!(validate_name(&'\u{0}'.to_string()), Err(_));
        assert_matches!(validate_name("./some"), Err(_));
        assert_matches!(validate_name("../../root"), Err(_));
        assert_matches!(validate_name("/"), Err(_));
        assert_matches!(validate_name("\u{1c}\u{7}"), Err(_));
        assert_matches!(validate_name("./foo/bar.txt"), Err(_));
        assert_matches!(validate_name("."), Err(_));
        assert_matches!(validate_name(".."), Err(_));
        assert_matches!(validate_name("/dev/sdb"), Err(_));
        assert_matches!(validate_name(""), Err(_));
        assert_matches!(validate_name("/"), Err(_));
        assert_matches!(validate_name(" leading_space"), Err(_));
        assert_matches!(validate_name("trailing_space "), Err(_));
        assert_matches!(validate_name("\u{0}leading_null"), Err(_));
        assert_matches!(validate_name("trailing_null\u{0}"), Err(_));
        assert_matches!(validate_name("exclamat!on"), Err(_));
        assert_matches!(validate_name("dollar$ign"), Err(_));
        assert_matches!(validate_name("middle\u{0}_null"), Err(_));
        assert_matches!(validate_name("\u{0}multiple\u{0}_null\u{0}"), Err(_));
        assert_matches!(validate_name(&"𐌏".repeat(64)), Err(_));

        assert_matches!(validate_name(&"𐌏".repeat(63)), Ok(_));
        assert_matches!(validate_name(&'\u{10fff8}'.to_string()), Ok(_));
        assert_matches!(validate_name("*< ? >"), Err(_));
        assert_matches!(validate_name("..."), Ok(_));
        assert_matches!(validate_name("ok.name"), Ok(_));
        assert_matches!(validate_name("ok name with spaces"), Err(_));
        assert_matches!(validate_name("\\\\"), Err(_));
        assert_matches!(validate_name("\u{211D}"), Ok(_));
        assert_matches!(validate_name("☺"), Ok(_));
        assert_matches!(validate_name("ok_name"), Ok(_));
        assert_matches!(validate_name("ユニコード"), Ok(_));
        assert_matches!(validate_name("ユニコード?"), Err(_));
    }
}
