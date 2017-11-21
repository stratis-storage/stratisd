// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

pub type DevUuid = Uuid;
pub type FilesystemUuid = Uuid;
pub type PoolUuid = Uuid;

#[derive(Debug, PartialEq, Eq)]
pub enum RenameAction {
    Identity,
    NoSource,
    Renamed,
}

/// See Design Doc section 10.2.1 for more details.
#[derive(Debug, PartialEq, Eq)]
pub enum BlockDevState {
    Missing,
    Bad,
    Spare,
    NotInUse,
    InUse,
}

/// Redundancy classifications which the engine allows for pools.
macro_attr! {
    #[derive(Debug, Eq, PartialEq)]
    #[allow(non_camel_case_types)]
    /// Redundancy specification for a pool.
    pub enum Redundancy {
        NONE,
    }
}

/// Get the u16 value of this Redundancy constructor.
impl From<Redundancy> for u16 {
    fn from(r: Redundancy) -> u16 {
        r as u16
    }
}
