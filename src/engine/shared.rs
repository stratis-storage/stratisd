// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    fs::File,
    io::Read,
    os::unix::io::{FromRawFd, RawFd},
    path::{Path, PathBuf},
};

use chrono::{DateTime, LocalResult, TimeZone, Utc};
use nix::poll::{poll, PollFd, PollFlags};
use regex::Regex;

use devicemapper::{Bytes, Sectors, IEC, SECTOR_SIZE};

use crate::{
    engine::{
        engine::{Pool, MAX_STRATIS_PASS_SIZE},
        types::{
            BlockDevTier, CreateAction, DevUuid, Diff, EncryptionInfo, MaybeInconsistent, Name,
            PoolEncryptionInfo, PoolUuid, SetCreateAction,
        },
    },
    stratis::{StratisError, StratisResult},
};

#[cfg(not(test))]
const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB
#[cfg(test)]
pub const DEFAULT_THIN_DEV_SIZE: Sectors = Sectors(2 * IEC::Gi); // 1 TiB

// Current versions of xfs now reject "small" filesystems:
// The data section of the filesystem must be at least 300 MiB.
// A Stratis imposed minimum of 512 MiB allows sufficient space for XFS
// metadata.
const MIN_THIN_DEV_SIZE: Sectors = Sectors(IEC::Mi); // 512 MiB

// Linux has a maximum filename length of 255 bytes. We use this length
// as a cap on the size of the pool name also. This ensures that the name
// serialized in the pool-level metadata has a bounded length.
const MAXIMUM_NAME_SIZE: usize = 255;

/// Called when the name of a requested pool coincides with the name of an
/// existing pool. Returns an error if the specifications of the requested
/// pool differ from the specifications of the existing pool, otherwise
/// returns Ok(CreateAction::Identity).
pub fn create_pool_idempotent_or_err<P>(
    pool: &P,
    pool_name: &Name,
    blockdev_paths: &[&Path],
) -> StratisResult<CreateAction<PoolUuid>>
where
    P: Pool,
{
    let input_devices: HashSet<PathBuf, RandomState> =
        blockdev_paths.iter().map(|p| p.to_path_buf()).collect();

    let existing_paths: HashSet<PathBuf, _> = pool
        .blockdevs()
        .iter()
        .filter_map(|(_, tier, bd)| {
            if *tier == BlockDevTier::Data {
                Some(bd.devnode().to_owned())
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
        Err(StratisError::Msg(create_pool_generate_error_string!(
            pool_name, in_input, in_pool
        )))
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
    let input_devices: HashSet<_> = blockdev_paths.iter().map(|p| p.to_path_buf()).collect();
    let existing_devices = existing_iter.collect();
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
        Err(StratisError::Msg(init_cache_generate_error_string!(
            in_input, in_pool
        )))
    }
}

/// Shared implementation of setting keys in the keyring for both the strat_engine
/// and sim_engine.
pub fn set_key_shared(key_fd: RawFd, memory: &mut [u8]) -> StratisResult<usize> {
    let mut key_file = unsafe { File::from_raw_fd(key_fd) };

    let bytes_read = key_file.read(memory)?;

    if bytes_read == MAX_STRATIS_PASS_SIZE {
        let mut pollers = [PollFd::new(&key_file, PollFlags::POLLIN)];
        let num_events = poll(&mut pollers, 0)?;
        if num_events > 0 {
            return Err(StratisError::Msg(format!(
                "Provided key exceeded maximum allow length of {}",
                Bytes::from(MAX_STRATIS_PASS_SIZE)
            )));
        }
    }

    Ok(bytes_read)
}

/// Validate a str for use as a Pool or Filesystem name.
pub fn validate_name(name: &str) -> StratisResult<()> {
    if name.is_empty() {
        return Err(StratisError::Msg(format!(
            "Provided string is empty: {name}"
        )));
    }

    if name.contains('\u{0}') {
        return Err(StratisError::Msg(format!(
            "Provided string contains NULL characters: {name}"
        )));
    }
    if name == "." || name == ".." {
        return Err(StratisError::Msg(format!("Name is . or .. : {name}")));
    }
    if name.len() > MAXIMUM_NAME_SIZE {
        return Err(StratisError::Msg(format!(
            "Provided string has more than {MAXIMUM_NAME_SIZE} bytes: {name}"
        )));
    }
    if name.len() != name.trim().len() {
        return Err(StratisError::Msg(format!(
            "Provided string contains leading or trailing space: {name}"
        )));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(StratisError::Msg(format!(
            "Provided string contains control characters: {name}"
        )));
    }
    lazy_static! {
        static ref NAME_UDEVREGEX: Regex =
            Regex::new(r"[[:ascii:]&&[^0-9A-Za-z#+-.:=@_/]]+").expect("regex is valid");
    }
    if NAME_UDEVREGEX.is_match(name) {
        return Err(StratisError::Msg(format!(
            "Provided string contains characters not allowed in udev symlinks: {name}"
        )));
    }

    let name_path = Path::new(name);
    if name_path.components().count() > 1 || name_path.is_absolute() {
        return Err(StratisError::Msg(format!(
            "Provided string is a directory path: {name}"
        )));
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
        Err(StratisError::Msg(format!(
            "Paths{{{}}} are not absolute",
            non_absolute_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

pub fn validate_filesystem_size(
    name: &str,
    size_opt: Option<Bytes>,
) -> StratisResult<Option<Sectors>> {
    size_opt
        .map(|size| {
            let size_sectors = size.sectors();
            if size_sectors.bytes() != size {
                Err(StratisError::Msg(format!(
                    "Requested size or size limit of filesystem {name} must be divisible by {SECTOR_SIZE}"
                )))
            } else if size_sectors < MIN_THIN_DEV_SIZE {
                Err(StratisError::Msg(format!(
                    "Requested size or size_limit of filesystem {name} is {size_sectors} which is less than minimum required: {MIN_THIN_DEV_SIZE}"
                )))
            } else {
                Ok(size_sectors)
            }
        })
        .transpose()
}

pub fn validate_filesystem_size_specs<'a>(
    specs: &[(&'a str, Option<Bytes>, Option<Bytes>)],
) -> StratisResult<HashMap<&'a str, (Sectors, Option<Sectors>)>> {
    specs
        .iter()
        .map(|&(name, size_opt, size_limit_opt)| {
            let size = validate_filesystem_size(name, size_opt)
                .map(|size_opt| size_opt.unwrap_or(DEFAULT_THIN_DEV_SIZE))?;
            let size_limit = validate_filesystem_size(name, size_limit_opt)?;
            Ok((name, (size, size_limit)))
        })
        .collect::<StratisResult<HashMap<_, (Sectors, Option<Sectors>)>>>()
}

/// Gather a collection of information from block devices that may or may not
/// be encrypted.
///
/// The Option type for the input iterator indicates whether or not a device is
/// encrypted. For encrypted devices, the iterator must return Some(_). For
/// unencrypted devices, the iterator must return None. A mixture of both
/// in the iterator will return an error.
fn gather<I, T, R, F>(len: usize, iterator: I, f: F) -> StratisResult<Option<R>>
where
    I: Iterator<Item = Option<T>>,
    F: Fn(Vec<T>) -> R,
{
    let infos = iterator.flatten().collect::<Vec<_>>();

    // Return error if not all devices are either encrypted or unencrypted.
    if infos.is_empty() {
        Ok(None)
    } else if infos.len() == len {
        Ok(Some(f(infos)))
    } else {
        Err(StratisError::Msg(
            "All devices in a pool must be either encrypted or unencrypted; found a mixture of both".to_string()
        ))
    }
}

/// Gather the encryption information from across multiple block devices.
pub fn gather_encryption_info<'a, I>(
    len: usize,
    iterator: I,
) -> StratisResult<Option<PoolEncryptionInfo>>
where
    I: Iterator<Item = Option<&'a EncryptionInfo>>,
{
    gather(len, iterator, PoolEncryptionInfo::from)
}

/// Gather the pool name information from across multiple block devices.
pub fn gather_pool_name<'a, I>(
    len: usize,
    iterator: I,
) -> StratisResult<Option<MaybeInconsistent<Option<Name>>>>
where
    I: Iterator<Item = Option<Option<&'a Name>>>,
{
    gather(len, iterator, |mut names| {
        let first_name = names.pop().expect("!names.is_empty()");
        names.into_iter().fold(
            MaybeInconsistent::No(first_name.cloned()),
            |name, next| match name {
                MaybeInconsistent::No(ref nopt) => {
                    if nopt.as_ref() == next {
                        name
                    } else {
                        MaybeInconsistent::Yes
                    }
                }
                MaybeInconsistent::Yes => MaybeInconsistent::Yes,
            },
        )
    })
}

/// Calculate the total used diff from the thin pool usage and metadata size.
pub fn total_used(used: &Diff<Option<Bytes>>, metadata_size: &Diff<Bytes>) -> Diff<Option<Bytes>> {
    let changed = matches!(
        (used, metadata_size),
        (Diff::Changed(_), _) | (Diff::Unchanged(Some(_)), Diff::Changed(_))
    );
    let total_used = used.map(|u| u + **metadata_size);
    if changed {
        Diff::Changed(total_used)
    } else {
        Diff::Unchanged(total_used)
    }
}

/// Calculate the allocated diff from a diff of the allocated size and metadata size.
pub fn total_allocated(allocated: &Diff<Bytes>, metadata_size: &Diff<Bytes>) -> Diff<Bytes> {
    match (allocated, metadata_size) {
        (Diff::Unchanged(a), Diff::Unchanged(m)) => Diff::Unchanged(*a + *m),
        (Diff::Changed(a), Diff::Unchanged(m)) => Diff::Changed(*a + *m),
        (Diff::Unchanged(a), Diff::Changed(m)) => Diff::Changed(*a + *m),
        (Diff::Changed(a), Diff::Changed(m)) => Diff::Changed(*a + *m),
    }
}

/// Convert a u64 value representing seconds, and a u32 value representing
/// nanoseconds to a timestamp.
pub fn unsigned_to_timestamp(secs: u64, nanos: u32) -> StratisResult<DateTime<Utc>> {
    let secs_arg = secs.try_into();
    match secs_arg {
        Ok(val) => match Utc.timestamp_opt(val, nanos) {
            LocalResult::Single(timestamp) => Ok(timestamp),
            _ => Err(StratisError::Msg(format!(
                "{val} (for seconds) and {nanos} (for nanoseconds) are not valid timestamp args"
            ))),
        },
        Err(_) => Err(StratisError::Msg(format!(
            "{secs} can not be converted into i64 to be used as seconds value in timestamp"
        ))),
    }
}

/// Return a timestamp which is equal to Utc::now() truncated to the nearest
/// second.
pub fn now_to_timestamp() -> DateTime<Utc> {
    Utc.timestamp_opt(Utc::now().timestamp(), 0).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
