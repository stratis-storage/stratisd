// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with device mapper devices.

use std::fmt;
use std::fmt::Display;

use devicemapper::ThinDevId;

use super::super::errors::EngineResult;

use super::super::super::engine::{FilesystemUuid, PoolUuid};

const FORMAT_VERSION: u16 = 1;

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


#[derive(Debug)]
/// A pool of thindev ids, all unique.
pub struct ThinDevIdPool {
    next_id: u32,
}

impl ThinDevIdPool {
    /// Make a new pool from a possibly empty Vec of ids.
    /// Does not verify the absence of duplicate ids.
    pub fn new_from_ids(ids: &[ThinDevId]) -> ThinDevIdPool {
        let max_id: Option<u32> = ids.into_iter().map(|x| (*x).into()).max();
        ThinDevIdPool { next_id: max_id.map(|x| x + 1).unwrap_or(0) }
    }

    /// Get a new id for a thindev.
    /// Returns an error if no thindev id can be constructed.
    // TODO: Improve this so that it is guaranteed only to fail if every 24 bit
    // number has been used.
    pub fn new_id(&mut self) -> EngineResult<ThinDevId> {
        let next_id = try!(ThinDevId::new_u64((self.next_id) as u64));
        self.next_id += 1;
        Ok(next_id)
    }
}
