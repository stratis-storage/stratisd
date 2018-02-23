// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with device mapper names.

use std::fmt;
use std::fmt::Display;

use devicemapper::{DmNameBuf, DmUuidBuf};

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
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 72
pub fn format_flex_ids(pool_uuid: PoolUuid, role: FlexRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!("stratis-{}-{}-flex-{}",
                        FORMAT_VERSION,
                        pool_uuid.simple().to_string(),
                        role);
    (DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display length < 72"),
     DmUuidBuf::new(value).expect("FORMAT_VERSION display length < 73"))

}

/// Format a name & uuid for the thin layer.
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 50
pub fn format_thin_ids(pool_uuid: PoolUuid, role: ThinRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!("stratis-{}-{}-thin-{}",
                        FORMAT_VERSION,
                        pool_uuid.simple().to_string(),
                        role);
    (DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display length < 50"),
     DmUuidBuf::new(value).expect("FORMAT_VERSION display length < 51"))
}

/// Format a name & uuid for the thin pool layer.
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 81
pub fn format_thinpool_ids(pool_uuid: PoolUuid, role: ThinPoolRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!("stratis-{}-{}-thinpool-{}",
                        FORMAT_VERSION,
                        pool_uuid.simple().to_string(),
                        role);
    (DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display_length < 81"),
     DmUuidBuf::new(value).expect("FORMAT_VERSION display_length < 82"))
}

/// Format a name & uuid for dm devices in the backstore.
/// Prerequisite: len(format!("{}", FORMAT_VERSION) < 76
pub fn format_backstore_ids(pool_uuid: PoolUuid, role: CacheRole) -> (DmNameBuf, DmUuidBuf) {
    let value = format!("stratis-{}-{}-physical-{}",
                        FORMAT_VERSION,
                        pool_uuid.simple().to_string(),
                        role);
    (DmNameBuf::new(value.clone()).expect("FORMAT_VERSION display_length < 76"),
     DmUuidBuf::new(value).expect("FORMAT_VERSION display_length < 77"))
}
