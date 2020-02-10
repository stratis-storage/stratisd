// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    path::{Path, PathBuf},
    vec::Vec,
};

use dbus::{
    arg::Array,
    tree::{MTFn, MethodInfo, MethodResult},
    Message,
};

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        pool::create_dbus_pool,
        types::TData,
        util::{
            engine_to_dbus_err_tuple, get_next_arg, msg_code_ok, msg_string_ok, tuple_to_option,
        },
    },
    engine::CreateAction,
};

/// Shared code for the creation of pools using the D-Bus API without the option
/// for a keyfile path or with an optional keyfile in later versions of the interface.
pub fn create_pool_shared(m: &MethodInfo<MTFn<TData>, TData>, has_keyfile: bool) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = get_next_arg(&mut iter, 0)?;
    let redundancy_tuple: (bool, u16) = get_next_arg(&mut iter, 1)?;
    let devs: Array<&str, _> = get_next_arg(&mut iter, 2)?;
    let keyfile_tuple: Option<(bool, &str)> = if has_keyfile {
        Some(get_next_arg(&mut iter, 3)?)
    } else {
        None
    };

    let object_path = m.path.get_name();
    let dbus_context = m.tree.get_data();
    let mut engine = dbus_context.engine.borrow_mut();
    let result = engine.create_pool(
        name,
        &devs.map(|x| Path::new(x)).collect::<Vec<&Path>>(),
        tuple_to_option(redundancy_tuple),
        keyfile_tuple.and_then(|kt| tuple_to_option(kt).map(PathBuf::from)),
    );

    let return_message = message.method_return();

    let default_return: (bool, (dbus::Path<'static>, Vec<dbus::Path<'static>>)) =
        (false, (dbus::Path::default(), Vec::new()));

    let msg = match result {
        Ok(pool_uuid_action) => {
            let results = match pool_uuid_action {
                CreateAction::Created(uuid) => {
                    let (_, pool) = get_mut_pool!(engine; uuid; default_return; return_message);

                    let pool_object_path: dbus::Path =
                        create_dbus_pool(dbus_context, object_path.clone(), uuid, pool);

                    let bd_paths = pool
                        .blockdevs_mut()
                        .into_iter()
                        .map(|(uuid, bd)| {
                            create_dbus_blockdev(dbus_context, pool_object_path.clone(), uuid, bd)
                        })
                        .collect::<Vec<_>>();
                    (true, (pool_object_path, bd_paths))
                }
                CreateAction::Identity => default_return,
            };
            return_message.append3(results, msg_code_ok(), msg_string_ok())
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&x);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}
