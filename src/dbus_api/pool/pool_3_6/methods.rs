// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::{arg::Array, Message};
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use devicemapper::Bytes;

use crate::{
    dbus_api::{
        filesystem::create_dbus_filesystem,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg, tuple_to_option},
    },
    engine::{EngineAction, Name},
};

type FilesystemSpec<'a> = (&'a str, (bool, &'a str), (bool, &'a str));

pub fn create_filesystems(m: &MethodInfo<'_, MTSync<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<'_, FilesystemSpec<'_>, _> = get_next_arg(&mut iter, 0)?;
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<(dbus::Path<'_>, &str)>) = (false, Vec::new());

    if filesystems.count() > 1 {
        let error_message = "only 1 filesystem per request allowed";
        let (rc, rs) = (DbusErrorEnum::ERROR as u16, error_message);
        return Ok(vec![return_message.append3(default_return, rc, rs)]);
    }

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

    let filesystem_specs = match filesystems
        .map(|(name, size_opt, size_limit_opt)| {
            let size = tuple_to_option(size_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!("Could not parse filesystem size string {val} to integer value")
                    })
                })
                .transpose()?;
            let size_limit = tuple_to_option(size_limit_opt)
                .map(|val| {
                    val.parse::<u128>().map_err(|_| {
                        format!(
                            "Could not parse filesystem size limit string {val} to integer value"
                        )
                    })
                })
                .transpose()?;
            Ok((name, size.map(Bytes), size_limit.map(Bytes)))
        })
        .collect::<Result<Vec<(&str, Option<Bytes>, Option<Bytes>)>, String>>()
    {
        Ok(val) => val,
        Err(err) => {
            let (rc, rs) = (DbusErrorEnum::ERROR as u16, err);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let result = handle_action!(
        pool.create_filesystems(&pool_name, pool_uuid, &filesystem_specs),
        dbus_context,
        pool_path.get_name()
    );

    let infos = match result {
        Ok(created_set) => created_set.changed(),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let return_value = match infos {
        Some(ref newly_created_filesystems) => {
            let v = newly_created_filesystems
                .iter()
                .map(|&(name, uuid, _)| {
                    let filesystem = pool
                        .get_filesystem(uuid)
                        .expect("just inserted by create_filesystems")
                        .1;
                    // FIXME: To avoid this expect, modify create_filesystem
                    // so that it returns a mutable reference to the
                    // filesystem created.
                    (
                        create_dbus_filesystem(
                            dbus_context,
                            object_path.clone(),
                            &pool_name,
                            &Name::new(name.to_string()),
                            uuid,
                            filesystem,
                        ),
                        name,
                    )
                })
                .collect::<Vec<_>>();
            (true, v)
        }
        None => default_return,
    };

    Ok(vec![return_message.append3(
        return_value,
        DbusErrorEnum::OK as u16,
        OK_STRING.to_string(),
    )])
}
