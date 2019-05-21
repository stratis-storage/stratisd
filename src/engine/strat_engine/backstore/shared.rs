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
    stratis::{ErrorEnum, StratisError, StratisResult},
};

/// Given a function that translates a Stratis UUID to a device
/// number, and some metadata that describes a particular segment within
/// a device by means of its Stratis UUID, and its start and offset w/in the
/// device, return the corresponding BlkDevSegment structure.
// This method necessarily takes a reference to a Box because it receives
// the value from a function, and there is no other way for the function to
// be returned from a closure.
// In future, it may be possible to address this better with FnBox.
#[allow(clippy::borrowed_box)]
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
