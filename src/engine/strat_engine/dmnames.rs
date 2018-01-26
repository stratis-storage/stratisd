// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with device mapper names.

use std::fmt;
use std::fmt::Display;

use devicemapper::DmNameBuf;

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

/// Format a name for the flex layer.
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 72
pub fn format_flex_name(pool_uuid: PoolUuid, role: FlexRole) -> DmNameBuf {
    DmNameBuf::new(format!("stratis-{}-{}-flex-{}",
                           FORMAT_VERSION,
                           pool_uuid.simple().to_string(),
                           role))
            .expect("FORMAT_VERSION display length < 72")

}

/// Format a name for the thin layer.
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 50
pub fn format_thin_name(pool_uuid: PoolUuid, role: ThinRole) -> DmNameBuf {
    DmNameBuf::new(format!("stratis-{}-{}-thin-{}",
                           FORMAT_VERSION,
                           pool_uuid.simple().to_string(),
                           role))
            .expect("FORMAT_VERSION display length < 50")
}

/// Format a name for the thin pool layer.
/// Prerequisite: len(format!("{}", FORMAT_VERSION)) < 81
pub fn format_thinpool_name(pool_uuid: PoolUuid, role: ThinPoolRole) -> DmNameBuf {
    DmNameBuf::new(format!("stratis-{}-{}-thinpool-{}",
                           FORMAT_VERSION,
                           pool_uuid.simple().to_string(),
                           role))
            .expect("FORMAT_VERSION display_length < 81")
}
