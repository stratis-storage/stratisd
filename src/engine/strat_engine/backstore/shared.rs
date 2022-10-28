// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Methods that are shared by the cache tier and the data tier.

use std::collections::HashMap;

use devicemapper::Device;

use crate::{
    engine::{
        strat_engine::{
            backstore::blockdevmgr::{BlkDevSegment, Segment},
            serde_structs::BaseDevSave,
        },
        types::DevUuid,
    },
    stratis::{StratisError, StratisResult},
};

/// Given a function that translates a Stratis UUID to a device
/// number, and some metadata that describes a particular segment within
/// a device by means of its Stratis UUID, and its start and offset w/in the
/// device, return the corresponding BlkDevSegment structure.
pub fn metadata_to_segment(
    uuid_to_devno: &HashMap<DevUuid, Device>,
    base_dev_save: &BaseDevSave,
) -> StratisResult<BlkDevSegment> {
    let parent = base_dev_save.parent;
    uuid_to_devno
        .get(&parent)
        .ok_or_else(|| {
            StratisError::Msg(format!(
                "No block device corresponding to stratisd UUID {:?} found",
                &parent
            ))
        })
        .map(|device| {
            BlkDevSegment::new(
                parent,
                Segment::new(*device, base_dev_save.start, base_dev_save.length),
            )
        })
}

/// Append the second list of BlkDevSegments to the first, or if the last
/// segment of the first argument is adjacent to the first segment of the
/// second argument, merge those two together.
/// Postcondition: left.len() + right.len() - 1 <= result.len()
/// Postcondition: result.len() <= left.len() + right.len()
pub fn coalesce_blkdevsegs(left: &[BlkDevSegment], right: &[BlkDevSegment]) -> Vec<BlkDevSegment> {
    left.iter().chain(right.iter()).cloned().fold(
        Vec::with_capacity(left.len() + right.len()),
        |mut collect, seg| {
            if let Some(left) = collect.last_mut() {
                if left.uuid == seg.uuid
                    && (left.segment.start + left.segment.length == seg.segment.start)
                {
                    left.segment.length += seg.segment.length;
                } else {
                    collect.push(seg);
                }
            } else {
                collect.push(seg);
            }
            collect
        },
    )
}
