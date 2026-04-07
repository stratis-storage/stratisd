// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::iter::IntoIterator;

use crate::{engine::strat_engine::serde_structs::FilesystemSave, stratis::StratisResult};

/// Define how an origin and its snapshot are merged when a filesystem is
/// reverted.
pub fn merge(origin: &FilesystemSave, snap: &FilesystemSave) -> FilesystemSave {
    FilesystemSave {
        name: origin.name.to_owned(),
        uuid: origin.uuid,
        thin_id: snap.thin_id,
        size: snap.size,
        created: origin.created,
        fs_size_limit: snap.fs_size_limit,
        origin: origin.origin,
        merge: origin.merge,
    }
}

pub fn shift_allocation_offset<'a, T: 'a>(
    iter: impl IntoIterator<Item = &'a T>,
    offset_map: impl Fn(&'a T) -> StratisResult<T>,
) -> StratisResult<Vec<T>> {
    iter.into_iter()
        .map(offset_map)
        .collect::<StratisResult<Vec<T>>>()
}
