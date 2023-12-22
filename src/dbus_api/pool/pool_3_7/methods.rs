// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use dbus::{arg::Array, Message};
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        consts::filesystem_interface_list,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{EngineAction, FilesystemUuid, StratisUuid},
};

pub fn destroy_filesystems(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<'_, dbus::Path<'static>, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<String>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = typed_uuid!(
        get_data!(pool_path; default_return; return_message).uuid;
        Pool;
        default_return;
        return_message
    );

    let mut guard = get_mut_pool!(dbus_context.engine; pool_uuid; default_return; return_message);
    let (pool_name, _, pool) = guard.as_mut_tuple();

    let mut filesystem_map: HashMap<FilesystemUuid, dbus::Path<'static>> = HashMap::new();
    for path in filesystems {
        if let Some((u, path)) = m.tree.get(&path).and_then(|op| {
            op.get_data()
                .as_ref()
                .map(|d| (&d.uuid, op.get_name().clone()))
        }) {
            let uuid = *typed_uuid!(u; Fs; default_return; return_message);
            filesystem_map.insert(uuid, path);
        }
    }

    let result = handle_action!(
        pool.destroy_filesystems(
            &pool_name,
            &filesystem_map.keys().cloned().collect::<Vec<_>>(),
        ),
        dbus_context,
        pool_path.get_name()
    );
    let msg = match result {
        Ok(uuids) => {
            // Only get changed values here as non-existent filesystems will have been filtered out
            // before calling destroy_filesystems
            let uuid_vec: Vec<String> =
                if let Some((ref changed_uuids, ref updated_uuids)) = uuids.changed() {
                    for uuid in changed_uuids {
                        let op = filesystem_map
                            .get(uuid)
                            .expect("'uuids' is a subset of filesystem_map.keys()");
                        dbus_context.push_remove(op, filesystem_interface_list());
                    }

                    for sn_op in m.tree.iter().filter(|op| {
                        op.get_data()
                            .as_ref()
                            .map(|data| match data.uuid {
                                StratisUuid::Fs(uuid) => updated_uuids.contains(&uuid),
                                _ => false,
                            })
                            .unwrap_or(false)
                    }) {
                        dbus_context.push_filesystem_origin_change(sn_op.get_name());
                    }

                    changed_uuids
                        .iter()
                        .map(|uuid| uuid_to_string!(uuid))
                        .collect()
                } else {
                    Vec::new()
                };
            return_message.append3(
                (true, uuid_vec),
                DbusErrorEnum::OK as u16,
                OK_STRING.to_string(),
            )
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}
