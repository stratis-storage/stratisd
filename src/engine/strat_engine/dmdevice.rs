// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with device mapper devices.

use std::fmt;
use std::fmt::Display;

use rand;
use serde;

use devicemapper::ThinDevId;

use super::super::super::engine::{FilesystemUuid, PoolUuid};

const FORMAT_VERSION: u16 = 1;

pub enum FlexRole {
    MetadataVolume,
    ThinData,
    ThinMeta,
}

impl Display for FlexRole {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FlexRole::MetadataVolume => write!(f, "mdv"),
            FlexRole::ThinData => write!(f, "thindata"),
            FlexRole::ThinMeta => write!(f, "thinmeta"),
        }
    }
}

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
pub fn format_flex_name(pool_uuid: &PoolUuid, role: FlexRole) -> String {
    return format!("stratis-{}-{}-flex-{}",
                   FORMAT_VERSION,
                   pool_uuid.simple().to_string(),
                   role);
}

/// Format a name for the thin layer.
pub fn format_thin_name(pool_uuid: &PoolUuid, role: ThinRole) -> String {
    return format!("stratis-{}-{}-thin-{}",
                   FORMAT_VERSION,
                   pool_uuid.simple().to_string(),
                   role);
}

/// Format a name for the thin pool layer.
pub fn format_thinpool_name(pool_uuid: &PoolUuid, role: ThinPoolRole) -> String {
    return format!("stratis-{}-{}-thinpool-{}",
                   FORMAT_VERSION,
                   pool_uuid.simple().to_string(),
                   role);
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SThinDevId {
    value: u32,
}

impl fmt::Display for SThinDevId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}

impl ThinDevId for SThinDevId {}

impl SThinDevId {
    /// Instantiate a new, random, thindev id.
    pub fn new_random() -> SThinDevId {
        SThinDevId { value: rand::random::<u32>() >> 8 }
    }
}

impl serde::Serialize for SThinDevId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: serde::Serializer
    {
        serializer.serialize_u32(self.value)
    }
}

impl<'de> serde::Deserialize<'de> for SThinDevId {
    fn deserialize<D>(deserializer: D) -> Result<SThinDevId, D::Error>
        where D: serde::de::Deserializer<'de>
    {
        Ok(SThinDevId { value: try!(serde::Deserialize::deserialize(deserializer)) })
    }
}
