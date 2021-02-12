// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with stratis and device mapper names.

use std::{
    convert::TryFrom,
    fmt::{self, Display},
};

use devicemapper::{DmNameBuf, DmUuidBuf};

pub use crate::engine::types::KeyDescription;
use crate::{
    engine::types::{DevUuid, FilesystemUuid, PoolUuid},
    stratis::StratisResult,
};

const FORMAT_VERSION: u16 = 1;

/// Prefix for the key descriptions added into the kernel keyring to indicate
/// that they were added by Stratis.
fn key_description_prefix() -> String {
    format!("stratis-{}-key-", FORMAT_VERSION)
}

impl KeyDescription {
    /// Check if the system key description has the Stratis prefix. If so,
    /// return `Some` with the prefix stripped. If not, return `None`.
    pub fn from_system_key_desc(raw_key_desc: &str) -> Option<StratisResult<KeyDescription>> {
        let mut key_desc = raw_key_desc.to_string();
        let prefix = key_description_prefix();
        if key_desc.starts_with(prefix.as_str()) {
            key_desc.replace_range(..prefix.len(), "");
            if key_desc.is_empty() {
                None
            } else {
                Some(KeyDescription::try_from(key_desc))
            }
        } else {
            None
        }
    }

    /// Return the key description as it will be registered on the system,
    /// not as it will be displayed in Stratis. The only difference between
    /// this and the application representation is the addition of a prefix
    /// that stratisd uses internally for keeping track of which keys belong
    /// to Stratis.
    pub fn to_system_string(&self) -> String {
        format!("{}{}", key_description_prefix(), self.as_application_str())
    }
}

/// Get a devicemapper name from the device UUID.
///
/// Prerequisite: len(format!("{}", FORMAT_VERSION)
///             + len("stratis")                         7
///             + len("private")                         7
///             + len("crypt")                           5
///             + num_dashes                             4
///             + len(dev uuid)                          32
///             < 128
///
/// which is equivalent to len(format!("{}", FORMAT_VERSION) < 73
pub fn format_crypt_name(dev_uuid: &DevUuid) -> String {
    format!(
        "stratis-{}-private-{}-crypt",
        FORMAT_VERSION,
        uuid_to_string!(dev_uuid)
    )
}

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
            ThinRole::Filesystem(uuid) => write!(f, "fs-{}", uuid_to_string!(uuid)),
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
    Cache,
    /// The cache sub-device of the DM cache device.
    CacheSub,
    /// The meta sub-device of the DM cache device.
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
        uuid_to_string!(pool_uuid),
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
        uuid_to_string!(pool_uuid),
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
        uuid_to_string!(pool_uuid),
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
        uuid_to_string!(pool_uuid),
        role
    );
    (
        DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display_length < 60"),
        DmUuidBuf::new(value).expect("FORMAT_VERSION display_length < 61"),
    )
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use super::*;

    #[test]
    fn test_key_desc() {
        assert!(KeyDescription::from_system_key_desc("stratis-1-key-").is_none());
        assert!(KeyDescription::from_system_key_desc("not-prefix-stratis-1-key-").is_none());
        assert_eq!(
            KeyDescription::from_system_key_desc("stratis-1-key-key_desc")
                .map(|k| k.expect("no semi-colons")),
            Some(KeyDescription::try_from("key_desc".to_string()).expect("no semi-colons"))
        );
        assert_eq!(
            KeyDescription::from_system_key_desc("stratis-1-key-stratis-1-key")
                .map(|k| k.expect("no semi-colons")),
            Some(KeyDescription::try_from("stratis-1-key".to_string()).expect("no semi-colons"))
        );
    }
}
