// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for interpreting and manipulation representation of DM structs.

use devicemapper::{ThinPoolStatus, ThinPoolStatusSummary};

/// A way of digesting the status reported on the thinpool into a value
/// that can be checked for equality. This way, two statuses,
/// collected at different times can be checked to determine whether their
/// gross, as opposed to fine, differences are significant.
/// In this implementation convert the status designations to strings which
/// match those strings that the kernel uses to identify the different states
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum_macros::AsRefStr)]
pub enum ThinPoolStatusDigest {
    #[strum(serialize = "Fail")]
    Fail,
    #[strum(serialize = "Error")]
    Error,
    #[strum(serialize = "rw")]
    Good,
    #[strum(serialize = "ro")]
    ReadOnly,
    #[strum(serialize = "out_of_data_space")]
    OutOfSpace,
}

impl From<&ThinPoolStatus> for ThinPoolStatusDigest {
    fn from(status: &ThinPoolStatus) -> ThinPoolStatusDigest {
        match status {
            ThinPoolStatus::Working(status) => match status.summary {
                ThinPoolStatusSummary::Good => ThinPoolStatusDigest::Good,
                ThinPoolStatusSummary::ReadOnly => ThinPoolStatusDigest::ReadOnly,
                ThinPoolStatusSummary::OutOfSpace => ThinPoolStatusDigest::OutOfSpace,
            },
            ThinPoolStatus::Fail => ThinPoolStatusDigest::Fail,
            ThinPoolStatus::Error => ThinPoolStatusDigest::Error,
        }
    }
}

pub mod thin_pool_status_parser {
    use devicemapper::{MetaBlocks, ThinPoolStatus, ThinPoolUsage};

    /// Convert the thin pool status to usage information.
    pub fn usage(status: &ThinPoolStatus) -> Option<&ThinPoolUsage> {
        if let ThinPoolStatus::Working(w) = status {
            Some(&w.usage)
        } else {
            None
        }
    }

    /// Convert the thin pool status to the metadata low water mark.
    pub fn meta_lowater(status: &ThinPoolStatus) -> Option<MetaBlocks> {
        if let ThinPoolStatus::Working(w) = status {
            w.meta_low_water.map(MetaBlocks)
        } else {
            None
        }
    }
}
