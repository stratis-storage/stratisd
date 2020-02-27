// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Methods that are shared by the cache tier and the data tier.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use devicemapper::Device;

use crate::{
    engine::{
        strat_engine::{
            backstore::blockdevmgr::{BlkDevSegment, Segment},
            serde_structs::BaseDevSave,
        },
        types::DevUuid,
    },
    stratis::{ErrorEnum, StratisError, StratisResult},
};

#[derive(Clone, Debug)]
pub enum BlockDevPath {
    Encrypted { physical: PathBuf, logical: PathBuf },
    Unencrypted(PathBuf),
}

impl BlockDevPath {
    pub fn physical_path(&self) -> &Path {
        match *self {
            BlockDevPath::Encrypted { ref physical, .. } => physical,
            BlockDevPath::Unencrypted(ref path) => path,
        }
    }

    pub fn metadata_path(&self) -> &Path {
        match *self {
            BlockDevPath::Encrypted { ref logical, .. } => logical,
            BlockDevPath::Unencrypted(ref path) => path,
        }
    }
}

/// Given a function that translates a Stratis UUID to a device
/// number, and some metadata that describes a particular segment within
/// a device by means of its Stratis UUID, and its start and offset w/in the
/// device, return the corresponding BlkDevSegment structure.
// This method necessarily takes a reference to a Box because it receives
// the value from a function, and there is no other way for the function to
// be returned from a closure.
// In future, it may be possible to address this better with FnBox.
pub fn metadata_to_segment(
    uuid_to_devno: &HashMap<DevUuid, Device>,
    base_dev_save: &BaseDevSave,
) -> StratisResult<BlkDevSegment> {
    let parent = base_dev_save.parent;
    uuid_to_devno
        .get(&parent)
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!(
                    "No block device corresponding to stratisd UUID {:?} found",
                    &parent
                ),
            )
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
// FIXME: There is a method that duplicates this algorithm called
// coalesce_segs. These methods should either be unified into a single method
// OR one should go away entirely in solution to:
// https://github.com/stratis-storage/stratisd/issues/762.
pub fn coalesce_blkdevsegs(left: &[BlkDevSegment], right: &[BlkDevSegment]) -> Vec<BlkDevSegment> {
    if left.is_empty() {
        return right.to_vec();
    }
    if right.is_empty() {
        return left.to_vec();
    }

    let mut segments = Vec::with_capacity(left.len() + right.len());
    segments.extend_from_slice(left);

    // Last existing and first new may be contiguous.
    let coalesced = {
        let right_first = right.first().expect("!right.is_empty()");
        let left_last = segments.last_mut().expect("!left.is_empty()");
        if left_last.uuid == right_first.uuid
            && (left_last.segment.start + left_last.segment.length == right_first.segment.start)
        {
            left_last.segment.length += right_first.segment.length;
            true
        } else {
            false
        }
    };

    if coalesced {
        segments.extend_from_slice(&right[1..]);
    } else {
        segments.extend_from_slice(right);
    }
    segments
}
