// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use dbus::Message;
use dbus_tree::{MTSync, MethodInfo, MethodResult};

use crate::{
    dbus_api::{
        pool::Engine,
        types::{DbusErrorEnum, TData, OK_STRING},
        util::{engine_to_dbus_err_tuple, get_next_arg},
    },
    engine::{
        total_allocated, total_used, BlockDev, DevUuid, Diff, EngineAction, GrowAction, Pool,
        StratisUuid,
    },
};

pub fn grow_physical<E>(m: &MethodInfo<'_, MTSync<TData<E>>, TData<E>>) -> MethodResult
where
    E: 'static + Engine,
{
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let dev_uuid_str: &str = get_next_arg(&mut iter, 0)?;
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = false;

    let dev_uuid = match DevUuid::parse_str(dev_uuid_str) {
        Ok(u) => u,
        Err(e) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&e);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

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

    let result = handle_action!(
        pool.grow_physical(&pool_name, pool_uuid, dev_uuid)
            .map(|(act, diff)| {
                if act.is_changed() {
                    if let Some(d) = diff {
                        dbus_context.push_pool_foreground_change(
                            pool_path.get_name(),
                            total_used(&d.thin_pool.used, &d.pool.metadata_size),
                            total_allocated(&d.thin_pool.allocated_size, &d.pool.metadata_size),
                            Diff::Changed(pool.total_physical_size().bytes()),
                            d.pool.out_of_alloc_space,
                        )
                    }
                }
                act
            }),
        dbus_context,
        pool_path.get_name()
    );

    let ret = match result {
        Ok(GrowAction::Identity) => false,
        Ok(GrowAction::Grown((_, dev_uuid))) => {
            match m.tree.iter().find(|op| {
                op.get_data()
                    .as_ref()
                    .map(|data| match data.uuid {
                        StratisUuid::Dev(uuid) => uuid == dev_uuid,
                        _ => false,
                    })
                    .unwrap_or(false)
            }) {
                Some(op) => {
                    let dev_total_physical_size = pool
                        .get_blockdev(dev_uuid)
                        .expect("dev_uuid is the UUID of the blockdev that was just expanded; it must be in pool")
                        .1
                        .size();
                    dbus_context.push_blockdev_total_physical_size_change(
                        op.get_name(),
                        dev_total_physical_size,
                    );
                }
                None => {
                    warn!("Could not find object path for blockdev uuid {dev_uuid}; no property changed signal requested");
                }
            }

            true
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    Ok(vec![return_message.append3(
        ret,
        DbusErrorEnum::OK as u16,
        OK_STRING.to_string(),
    )])
}
