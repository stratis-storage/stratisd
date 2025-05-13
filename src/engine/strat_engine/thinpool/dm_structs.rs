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
    use devicemapper::{DataBlocks, MetaBlocks, ThinPoolStatus};

    /// Convert the thin pool status to the metadata low water mark.
    pub fn meta_lowater(status: &ThinPoolStatus) -> Option<MetaBlocks> {
        if let ThinPoolStatus::Working(w) = status {
            w.meta_low_water.map(MetaBlocks)
        } else {
            None
        }
    }

    /// Convert the thin pool status to the used information.
    pub fn used(status: &ThinPoolStatus) -> Option<(DataBlocks, MetaBlocks)> {
        if let ThinPoolStatus::Working(w) = status {
            Some((w.usage.used_data, w.usage.used_meta))
        } else {
            None
        }
    }
}

pub mod thin_table {
    use std::collections::HashSet;

    use devicemapper::ThinPoolDevTargetTable;

    /// Get the set of feature args.
    pub fn get_feature_args(table: &ThinPoolDevTargetTable) -> &HashSet<String> {
        &table.table.params.feature_args
    }
}

pub mod linear_table {

    use devicemapper::{
        Device, FlakeyTargetParams, LinearDevTargetParams, LinearDevTargetTable,
        LinearTargetParams, Sectors, TargetLine,
    };

    use crate::engine::types::OffsetDirection;

    /// Transform a list of segments belonging to a single device into a
    /// list of target lines for a linear device.
    pub fn segs_to_table(
        dev: Device,
        segments: &[(Sectors, Sectors)],
    ) -> Vec<TargetLine<LinearDevTargetParams>> {
        let mut table = Vec::new();
        let mut logical_start_offset = Sectors(0);

        for &(start_offset, length) in segments {
            let params = LinearTargetParams::new(dev, start_offset);
            let line = TargetLine::new(
                logical_start_offset,
                length,
                LinearDevTargetParams::Linear(params),
            );
            table.push(line);
            logical_start_offset += length;
        }
        table
    }

    /// Set the device on all lines in a linear table.
    pub fn set_target_device(
        table: &LinearDevTargetTable,
        device: Device,
        offset: Sectors,
        offset_direction: OffsetDirection,
    ) -> Vec<TargetLine<LinearDevTargetParams>> {
        let xform_target_line = |line: &TargetLine<LinearDevTargetParams>,
                                 offset,
                                 offset_direction|
         -> TargetLine<LinearDevTargetParams> {
            let new_params = match line.params {
                LinearDevTargetParams::Linear(ref params) => {
                    LinearDevTargetParams::Linear(LinearTargetParams::new(
                        device,
                        match offset_direction {
                            OffsetDirection::Forwards => params.start_offset + offset,
                            OffsetDirection::Backwards => params.start_offset - offset,
                        },
                    ))
                }
                LinearDevTargetParams::Flakey(ref params) => {
                    let feature_args = params.feature_args.iter().cloned().collect::<Vec<_>>();
                    LinearDevTargetParams::Flakey(FlakeyTargetParams::new(
                        device,
                        match offset_direction {
                            OffsetDirection::Forwards => params.start_offset + offset,
                            OffsetDirection::Backwards => params.start_offset - offset,
                        },
                        params.up_interval,
                        params.down_interval,
                        feature_args,
                    ))
                }
            };

            TargetLine::new(line.start, line.length, new_params)
        };

        table
            .table
            .clone()
            .iter()
            .map(|line| xform_target_line(line, offset, offset_direction))
            .collect::<Vec<_>>()
    }
}
