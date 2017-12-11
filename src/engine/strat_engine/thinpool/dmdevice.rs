// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with device mapper devices.

use std::fmt;
use std::fmt::Display;

use devicemapper::{DmNameBuf, ThinDevId};

use super::super::super::errors::EngineResult;

use super::super::super::super::engine::{FilesystemUuid, PoolUuid};

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
        let next_id = ThinDevId::new_u64(u64::from(self.next_id))?;
        self.next_id += 1;
        Ok(next_id)
    }
}
