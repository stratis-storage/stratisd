// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use uuid::Uuid;

pub type DevUuid = Uuid;
pub type FilesystemUuid = Uuid;
pub type PoolUuid = Uuid;

#[derive(Debug)]
pub enum RenameAction {
    Identity,
    NoSource,
    Renamed,
}

/// Redundancy classifications which the engine allows for pools.
custom_derive! {
    #[derive(Debug, Eq, PartialEq, EnumDisplay,
             IterVariants(RedundancyVariants))]
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
