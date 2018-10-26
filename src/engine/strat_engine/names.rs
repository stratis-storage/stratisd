// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with stratis and device mapper names.

use std::fmt;
use std::fmt::Display;
use std::path::Path;

use devicemapper::{DmNameBuf, DmUuidBuf};

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::super::engine::{FilesystemUuid, PoolUuid};

const FORMAT_VERSION: u16 = 1;

#[derive(Clone, Copy)]
pub enum FlexRole {
    MetadataVolume,
    ThinData,
    ThinMeta,
    ThinMetaSpare,
}

impl Display for FlexRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FlexRole::MetadataVolume => write!(f, "mdv"),
            FlexRole::ThinData => write!(f, "thindata"),
            FlexRole::ThinMeta => write!(f, "thinmeta"),
            FlexRole::ThinMetaSpare => write!(f, "thinmetaspare"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum ThinRole {
    Filesystem(FilesystemUuid),
}

impl Display for ThinRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ThinRole::Filesystem(uuid) => write!(f, "fs-{}", uuid.simple().to_string()),
        }
    }
}

#[derive(Clone, Copy)]
pub enum ThinPoolRole {
    Pool,
}

impl Display for ThinPoolRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ThinPoolRole::Pool => write!(f, "pool"),
        }
    }
}

/// The various roles taken on by DM devices in the cache tier.
#[derive(Clone, Copy)]
pub enum CacheRole {
    /// The DM cache device, contains the other three devices.
    #[allow(dead_code)]
    Cache,
    /// The cache sub-device of the DM cache device.
    #[allow(dead_code)]
    CacheSub,
    /// The meta sub-device of the DM cache device.
    #[allow(dead_code)]
    MetaSub,
    /// The origin sub-device of the DM cache device, holds the actual data.
    OriginSub,
}

impl Display for CacheRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CacheRole::Cache => write!(f, "cache"),
            CacheRole::CacheSub => write!(f, "cachesub"),
            CacheRole::MetaSub => write!(f, "metasub"),
            CacheRole::OriginSub => write!(f, "originsub"),
        }
    }
}

/// Format a name & uuid for the flex layer.
///
/// Prerequisite: len(format!("{}", FORMAT_VERSION)
///             + len("stratis")                         7
///             + len("private")                         7
///             + len("flex")                            4
///             + num_dashes                             5
///             + len(pool uuid)                         32
///             + max(len(FlexRole))                     13
///             < 128 (129 for UUID)
///
/// which is equivalent to len(format!("{}", FORMAT_VERSION) < 60 (61 for UUID)
pub fn format_flex_ids(pool_uuid: PoolUuid, role: FlexRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!(
        "stratis-{}-private-{}-flex-{}",
        FORMAT_VERSION,
        pool_uuid.simple().to_string(),
        role
    );
    (
        DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display length < 60"),
        DmUuidBuf::new(value).expect("FORMAT_VERSION display length < 61"),
    )
}

/// Format a name & uuid for the thin layer.
///
/// Prerequisite: len(format!("{}", FORMAT_VERSION)
///             + len("stratis")                         7
///             + len("thin")                            4
///             + num_dashes                             4
///             + len(pool uuid)                         32
///             + max(len(ThinRole))                     35
///             < 128 (129 for UUID)
///
/// which is equivalent to len(format!("{}", FORMAT_VERSION) < 46 (47 for UUID)
pub fn format_thin_ids(pool_uuid: PoolUuid, role: ThinRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!(
        "stratis-{}-{}-thin-{}",
        FORMAT_VERSION,
        pool_uuid.simple().to_string(),
        role
    );
    (
        DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display length < 46"),
        DmUuidBuf::new(value).expect("FORMAT_VERSION display length < 47"),
    )
}

/// Format a name & uuid for the thin pool layer.
///
/// Prerequisite: len(format!("{}", FORMAT_VERSION)
///             + len("stratis")                         7
///             + len("private")                         7
///             + len("thinpool")                        8
///             + num_dashes                             5
///             + len(pool uuid)                         32
///             + max(len(ThinPoolRole))                 4
///             < 128 (129 for UUID)
///
/// which is equivalent to len(format!("{}", FORMAT_VERSION) < 65 (66 for UUID)
pub fn format_thinpool_ids(pool_uuid: PoolUuid, role: ThinPoolRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!(
        "stratis-{}-private-{}-thinpool-{}",
        FORMAT_VERSION,
        pool_uuid.simple().to_string(),
        role
    );
    (
        DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display_length < 65"),
        DmUuidBuf::new(value).expect("FORMAT_VERSION display_length < 66"),
    )
}

/// Format a name & uuid for dm devices in the backstore.
///
/// Prerequisite: len(format!("{}", FORMAT_VERSION)
///             + len("stratis")                         7
///             + len("private")                         7
///             + len("physical")                        8
///             + num_dashes                             5
///             + len(pool uuid)                         32
///             + max(len(CacheRole))                    9
///             < 128 (129 for UUID)
///
/// which is equivalent to len(format!("{}", FORMAT_VERSION) < 60 (61 for UUID)
pub fn format_backstore_ids(pool_uuid: PoolUuid, role: CacheRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!(
        "stratis-{}-private-{}-physical-{}",
        FORMAT_VERSION,
        pool_uuid.simple().to_string(),
        role
    );
    (
        DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display_length < 60"),
        DmUuidBuf::new(value).expect("FORMAT_VERSION display_length < 61"),
    )
}

/// Validate a path for use as a Pool or Filesystem name.
pub fn validate_name(name: &str) -> StratisResult<()> {
    let name_path = Path::new(name);
    if name.contains('\u{0}') {
        return Err(StratisError::Engine(
            ErrorEnum::Invalid,
            format!("Name contains NULL characters : {}", name),
        ));
    }
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
            format!("Name has more than 255 characters : {}", name),
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
    Ok(())
}

#[cfg(test)]
mod tests {

    use engine::strat_engine::names::validate_name;

    #[test]
    pub fn test_validate_name() {
        assert!(validate_name(&'\u{0}'.to_string()).is_err());
        assert!(validate_name("./some").is_err());
        assert!(validate_name("../../root").is_err());
        assert!(validate_name("/").is_err());
        assert!(validate_name("\u{1c}\u{7}").is_err());
        assert!(validate_name("./foo/bar.txt").is_err());
        assert!(validate_name(".").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("/dev/sdb").is_err());
        assert!(validate_name("").is_err());
        assert!(validate_name("/").is_err());
        assert!(validate_name(" leading_space").is_err());
        assert!(validate_name("trailing_space ").is_err());
        assert!(validate_name("\u{0}leading_null").is_err());
        assert!(validate_name("trailing_null\u{0}").is_err());
        assert!(validate_name("middle\u{0}_null").is_err());
        assert!(validate_name("\u{0}multiple\u{0}_null\u{0}").is_err());
        assert!(validate_name(&"𐌏".repeat(64)).is_err());

        assert!(validate_name(&"𐌏".repeat(63)).is_ok());
        assert!(validate_name(&'\u{10fff8}'.to_string()).is_ok());
        assert!(validate_name("*< ? >").is_ok());
        assert!(validate_name("...").is_ok());
        assert!(validate_name("ok.name").is_ok());
        assert!(validate_name("ok name with spaces").is_ok());
        assert!(validate_name("\\\\").is_ok());
        assert!(validate_name("\u{211D}").is_ok());
        assert!(validate_name("☺").is_ok());
        assert!(validate_name("ok_name").is_ok());
    }
}
