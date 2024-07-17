// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{Iter, IterAppend};
use dbus_tree::{MTSync, MethodErr, PropInfo};

use crate::{
    dbus_api::{
        consts,
        filesystem::shared::{self, get_filesystem_property},
        types::TData,
    },
    engine::PropChangeAction,
};

pub fn get_fs_origin(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, _, f)| Ok(shared::fs_origin_prop(f)))
}

pub fn get_fs_merge_scheduled(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, _, f)| Ok(shared::fs_merge_scheduled_prop(f)))
}

/// Set the merge scheduled property on a filesystem
pub fn set_fs_merge_scheduled(
    i: &mut Iter<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    let merge_scheduled: bool = i
        .get()
        .ok_or_else(|| MethodErr::failed("Value required as argument to set property"))?;

    let res = shared::set_fs_property_to_display(
        p,
        consts::FILESYSTEM_MERGE_SCHEDULED_PROP,
        |(_, uuid, p)| shared::set_fs_merge_scheduled_prop(uuid, p, merge_scheduled),
    );

    match res {
        Ok(PropChangeAction::NewValue(v)) => {
            p.tree
                .get_data()
                .push_fs_merge_scheduled_change(p.path.get_name(), v);
            Ok(())
        }
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(e),
    }
}
