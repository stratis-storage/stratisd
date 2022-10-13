// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::ops::{Deref, DerefMut};

use devicemapper::{Bytes, Sectors};

/// This interface defines a generic way to compare whether two values of
/// the same type have changed or remained the same.
pub trait Compare {
    /// Compare two values. `self` should always be the starting value and `other`
    /// should always be the newest value.
    fn compare(&self, other: &Self) -> Diff<Self>
    where
        Self: Sized;
}

impl<T> Compare for T
where
    T: PartialEq + Clone + Sized,
{
    fn compare(&self, other: &Self) -> Diff<Self> {
        if self != other {
            Diff::Changed(other.clone())
        } else {
            Diff::Unchanged(other.clone())
        }
    }
}

/// A type that represents whether the value contained inside is changed or unchanged
/// from the last time it was checked. This data structure retains the value in both
/// cases in case it is needed for other calculations.
#[derive(Debug)]
pub enum Diff<T> {
    Changed(T),
    Unchanged(T),
}

impl<T> Diff<T> {
    /// Determines whether the contained value was changed or unchanged.
    pub fn is_changed(&self) -> bool {
        matches!(self, Diff::Changed(_))
    }

    /// Return the changed variant or None if it is unchanged.
    pub fn changed(self) -> Option<T> {
        match self {
            Diff::Changed(c) => Some(c),
            Diff::Unchanged(_) => None,
        }
    }
}

impl<T> Deref for Diff<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Diff::Changed(c) => c,
            Diff::Unchanged(u) => u,
        }
    }
}

impl<T> DerefMut for Diff<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Diff::Changed(c) => c,
            Diff::Unchanged(u) => u,
        }
    }
}

/// Change in attributes of the thin pool that may need to be reported to the
/// IPC layer.
#[derive(Debug)]
pub struct ThinPoolDiff {
    pub allocated_size: Diff<Bytes>,
    pub used: Diff<Option<Bytes>>,
}

/// Change in attributes of a Stratis pool that may need to be reported to the
/// IPC layer.
#[derive(Debug)]
pub struct StratPoolDiff {
    pub metadata_size: Diff<Bytes>,
    pub out_of_alloc_space: Diff<bool>,
}

/// Represents the difference between two dumped states for a filesystem.
#[derive(Debug)]
pub struct StratFilesystemDiff {
    pub size: Diff<Bytes>,
    pub used: Diff<Option<Bytes>>,
}

/// Represents the difference between two dumped states for a pool.
pub struct PoolDiff {
    pub thin_pool: ThinPoolDiff,
    pub pool: StratPoolDiff,
}

/// Represents the difference between two dumped states for a block device.
#[derive(Debug)]
pub struct StratBlockDevDiff {
    pub size: Diff<Option<Sectors>>,
}
