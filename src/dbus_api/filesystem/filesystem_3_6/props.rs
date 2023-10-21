// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::arg::{Iter, IterAppend};
use dbus_tree::{MTSync, MethodErr, PropInfo};

use devicemapper::Bytes;

use crate::{
    dbus_api::{
        consts,
        filesystem::shared::{self, get_filesystem_property},
        types::TData,
        util::tuple_to_option,
    },
    engine::PropChangeAction,
};

pub fn get_fs_size_limit(
    i: &mut IterAppend<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, _, f)| Ok(shared::fs_size_limit_prop(f)))
}

pub fn set_fs_size_limit(
    i: &mut Iter<'_>,
    p: &PropInfo<'_, MTSync<TData>, TData>,
) -> Result<(), MethodErr> {
    let size_limit_opt: (bool, &str) = i.get().ok_or_else(|| {
        MethodErr::failed("New filesystem limit required as argument to increase it")
    })?;
    let size_limit_str = tuple_to_option(size_limit_opt);
    let size_limit = match size_limit_str {
        Some(lim) => Some(Bytes(lim.parse::<u128>().map_err(|e| {
            MethodErr::failed(&format!("Failed to parse {lim} as unsigned integer: {e}"))
        })?)),
        None => None,
    };

    let res = shared::set_fs_property_to_display(
        p,
        consts::FILESYSTEM_SIZE_LIMIT_PROP,
        |(_, uuid, p)| shared::set_fs_size_limit_prop(uuid, p, size_limit),
    );
    match res {
        Ok(PropChangeAction::NewValue(v)) => {
            p.tree
                .get_data()
                .push_fs_size_limit_change(p.path.get_name(), v);
            Ok(())
        }
        Ok(PropChangeAction::Identity) => Ok(()),
        Err(e) => Err(e),
    }
}
