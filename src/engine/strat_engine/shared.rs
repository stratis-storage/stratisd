// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::engine::strat_engine::serde_structs::FilesystemSave;

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
