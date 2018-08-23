// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

#[cfg(feature = "dbus_enabled")]
use dbus;
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDevState {
    Missing,
    Bad,
    Spare,
    NotInUse,
    InUse,
}

impl BlockDevState {
    pub fn to_dbus_value(&self) -> u16 {
        match self {
            BlockDevState::Missing => 0,
            BlockDevState::Bad => 1,
            BlockDevState::Spare => 2,
            BlockDevState::NotInUse => 3,
            BlockDevState::InUse => 4,
        }
    }
}

/// A struct that may contain a dbus::Path, or may not, and most certainly
/// doesn't if dbus is compiled out. This avoids littering engine code with
/// conditional code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaybeDbusPath(
    #[cfg(feature = "dbus_enabled")] pub Option<dbus::Path<'static>>,
    #[cfg(not(feature = "dbus_enabled"))] pub Option<()>,
);

/// Blockdev tier. Used to distinguish between blockdevs used for
/// data and blockdevs used for a cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockDevTier {
    Data,
    Cache,
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

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Name(Rc<String>);

impl Name {
    pub fn new(name: String) -> Name {
        Name(Rc::new(name))
    }

    pub fn to_owned(&self) -> String {
        self.0.deref().to_owned()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Deref for Name {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &**self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
