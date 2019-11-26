// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, path::Path, vec::Vec};

use dbus::{
    self,
    arg::{Array, IterAppend, RefArg, Variant},
    tree::{
        Access, EmitsChangedSignal, Factory, MTFn, Method, MethodErr, MethodInfo, MethodResult,
        PropInfo, Property, Tree,
    },
    Message,
};
use itertools::Itertools;

use devicemapper::Sectors;

use crate::{
    dbus_api::{
        blockdev::create_dbus_blockdev,
        consts,
        filesystem::create_dbus_filesystem,
        types::{DbusContext, DbusErrorEnum, OPContext, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, get_uuid, make_object_path, msg_code_ok,
            msg_string_ok, result_to_tuple,
        },
    },
    engine::{
        BlockDevTier, CreateAction, EngineAction, FilesystemUuid, MaybeDbusPath, Name, Pool,
        PoolUuid, RenameAction,
    },
};

fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<&str, _> = get_next_arg(&mut iter, 0)?;
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<(dbus::Path, &str)>) = (false, Vec::new());

    if filesystems.count() > 1 {
        let error_message = "only 1 filesystem per request allowed";
        let (rc, rs) = (DbusErrorEnum::ERROR as u16, error_message);
        return Ok(vec![return_message.append3(default_return, rc, rs)]);
    }

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let result = pool.create_filesystems(
        pool_uuid,
        &pool_name,
        &filesystems
            .map(|x| (x, None))
            .collect::<Vec<(&str, Option<Sectors>)>>(),
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
                .map(|&(name, uuid)| {
                    // FIXME: To avoid this expect, modify create_filesystem
                    // so that it returns a mutable reference to the
                    // filesystem created.
                    (
                        create_dbus_filesystem(
                            dbus_context,
                            object_path.clone(),
                            uuid,
                            pool.get_mut_filesystem(uuid)
                                .expect("just inserted by create_filesystems")
                                .1,
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
        msg_code_ok(),
        msg_string_ok(),
    )])
}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<dbus::Path<'static>, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<String>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let filesystem_map: HashMap<FilesystemUuid, dbus::Path<'static>> = filesystems
        .filter_map(|path| {
            m.tree.get(&path).and_then(|op| {
                op.get_data()
                    .as_ref()
                    .map(|d| (d.uuid, op.get_name().clone()))
            })
        })
        .collect();

    let result = pool.destroy_filesystems(
        &pool_name,
        &filesystem_map.keys().cloned().collect::<Vec<_>>(),
    );
    let msg = match result {
        Ok(uuids) => {
            // Only get changed values here as non-existant filesystems will have been filtered out
            // before calling destroy_filesystems
            let uuid_vec: Vec<String> = if let Some(ref changed_uuids) = uuids.changed() {
                for uuid in changed_uuids {
                    let op = filesystem_map
                        .get(uuid)
                        .expect("'uuids' is a subset of filesystem_map.keys()");
                    dbus_context.actions.borrow_mut().push_remove(op, m.tree);
                }
                changed_uuids
                    .iter()
                    .map(|uuid| uuid_to_string!(uuid))
                    .collect()
            } else {
                Vec::new()
            };
            return_message.append3((true, uuid_vec), msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn snapshot_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystem: dbus::Path<'static> = get_next_arg(&mut iter, 0)?;
    let snapshot_name: &str = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, dbus::Path::default());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let fs_uuid = match m.tree.get(&filesystem) {
        Some(op) => get_data!(op; default_return; return_message).uuid,
        None => {
            let message = format!("no data for object path {}", filesystem);
            let (rc, rs) = (DbusErrorEnum::NOTFOUND as u16, message);
            return Ok(vec![return_message.append3(default_return, rc, rs)]);
        }
    };

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let msg = match pool.snapshot_filesystem(pool_uuid, &pool_name, fs_uuid, snapshot_name) {
        Ok(CreateAction::Created((uuid, fs))) => {
            let fs_object_path: dbus::Path =
                create_dbus_filesystem(dbus_context, object_path.clone(), uuid, fs);
            return_message.append3((true, fs_object_path), msg_code_ok(), msg_string_ok())
        }
        Ok(CreateAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn add_blockdevs(m: &MethodInfo<MTFn<TData>, TData>, tier: BlockDevTier) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<dbus::Path>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let result = pool.add_blockdevs(pool_uuid, &*pool_name, &blockdevs, tier);
    let msg = match result.map(|bds| bds.changed()) {
        Ok(Some(uuids)) => {
            let return_value = uuids
                .iter()
                .map(|uuid| {
                    // FIXME: To avoid this expect, modify add_blockdevs
                    // so that it returns a mutable reference to each
                    // blockdev created.
                    create_dbus_blockdev(
                        dbus_context,
                        object_path.clone(),
                        *uuid,
                        pool.get_mut_blockdev(*uuid)
                            .expect("just inserted by add_blockdevs")
                            .1,
                    )
                })
                .collect::<Vec<_>>();

            return_message.append3((true, return_value), msg_code_ok(), msg_string_ok())
        }
        Ok(None) => return_message.append3(default_return, msg_code_ok(), msg_string_ok()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn add_datadevs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevTier::Data)
}

fn add_cachedevs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    add_blockdevs(m, BlockDevTier::Cache)
}

fn add_cachedevs_2_1(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let return_message = message.method_return();
    let object_path = m.path.get_name();
    let default_return: (bool, Vec<dbus::Path>) = (false, Vec::new());
    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;
    let dbus_context = m.tree.get_data();
    let mut engine = dbus_context.engine.borrow_mut();
    let (_, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);
    if pool.cache_initialized() {
        add_blockdevs(m, BlockDevTier::Cache)
    } else {
        init_cache_2_1(m)
    }
}

fn init_cache_2_1(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = get_next_arg(&mut iter, 1)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return: (bool, Vec<dbus::Path>) = (false, Vec::new());

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (_, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();
    let result = pool.init_cache(
        pool_uuid,
        &blockdevs,
        pool.keyfile_path().map(|kfp| kfp.to_path_buf()),
    );
    let return_value = match result.map(|r| r.changed()) {
        Ok(Some(uuids)) => {
            let return_value = uuids
                .iter()
                .map(|uuid| {
                    // FIXME: To avoid this expect, modify add_blockdevs
                    // so that it returns a mutable reference to each
                    // blockdev created.
                    create_dbus_blockdev(
                        dbus_context,
                        object_path.clone(),
                        *uuid,
                        pool.get_mut_blockdev(*uuid)
                            .expect("just inserted by add_blockdevs")
                            .1,
                    )
                })
                .collect::<Vec<_>>();
            return_message.append3((true, return_value), msg_code_ok(), msg_string_ok())
        }
        Ok(None) => return_message.append3(default_return, msg_code_ok(), msg_string_ok()),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![return_value])
}

fn rename_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(PoolUuid::nil()));

    let pool_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let msg = match dbus_context
        .engine
        .borrow_mut()
        .rename_pool(pool_uuid, new_name)
    {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about pool {}", pool_uuid);
            let (rc, rs) = (DbusErrorEnum::INTERNAL_ERROR as u16, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            return_message.append3(default_return, msg_code_ok(), msg_string_ok())
        }
        Ok(RenameAction::Renamed(uuid)) => return_message.append3(
            (true, uuid_to_string!(uuid)),
            msg_code_ok(),
            msg_string_ok(),
        ),
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err_tuple(&err);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn pool_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
    object_path: &dbus::Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, PoolUuid, &dyn Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let pool_path = tree
        .get(object_path)
        .expect("implicit argument must be in tree");

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (pool_name, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;

    closure((pool_name, pool_uuid, pool))
}

/// Get a pool property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Pool and obtains the property from the pool.
fn get_pool_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, PoolUuid, &dyn Pool)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        pool_operation(p.tree, p.path.get_name(), getter).map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

fn get_pool_name(i: &mut IterAppend, p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    get_pool_property(i, p, |(name, _, _)| Ok(name.to_owned()))
}

fn get_properties_shared(
    m: &MethodInfo<MTFn<TData>, TData>,
    properties: &mut dyn Iterator<Item = String>,
) -> MethodResult {
    let message: &Message = m.msg;
    let object_path = &m.path;

    let return_message = message.method_return();

    let return_value: HashMap<String, (bool, Variant<Box<dyn RefArg>>)> = properties
        .unique()
        .filter_map(|prop| match prop.as_str() {
            consts::POOL_TOTAL_SIZE_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    Ok((u128::from(*pool.total_physical_size())
                        * devicemapper::SECTOR_SIZE as u128)
                        .to_string())
                }),
            )),
            consts::POOL_TOTAL_USED_PROP => Some((
                prop,
                pool_operation(m.tree, object_path.get_name(), |(_, _, pool)| {
                    pool.total_physical_used()
                        .map_err(|e| e.to_string())
                        .map(|size| {
                            (u128::from(*size) * devicemapper::SECTOR_SIZE as u128).to_string()
                        })
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| result_to_tuple(key, result))
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

fn get_all_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_properties_shared(
        m,
        &mut vec![consts::POOL_TOTAL_SIZE_PROP, consts::POOL_TOTAL_USED_PROP]
            .into_iter()
            .map(|s| s.to_string()),
    )
}

fn get_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    let mut properties: Array<String, _> = get_next_arg(&mut iter, 0)?;
    get_properties_shared(m, &mut properties)
}

pub fn create_dbus_pool<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: PoolUuid,
    pool: &mut dyn Pool,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let init_cache_method_2_1 = f
        .method("InitCache", (), init_cache_2_1)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_cachedevs_method = f
        .method("AddCacheDevs", (), add_cachedevs)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_cachedevs_method_2_1 = f
        .method("AddCacheDevs", (), add_cachedevs_2_1)
        .in_arg(("devices", "as"))
        // b: Indicates if any cache devices were added
        // ao: Array of object paths of created cache devices
        //
        // Rust representation: (bool, Vec<dbus::path>)
        .out_arg(("results", "(bao)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    fn create_filesystems_method_object(
        f: &Factory<MTFn<TData>, TData>,
    ) -> Method<MTFn<TData>, TData> {
        f.method("CreateFilesystems", (), create_filesystems)
            .in_arg(("specs", "as"))
            // b: true if filesystems were created
            // a(os): Array of tuples with object paths and names
            //
            // Rust representation: (bool, Vec<(dbus::Path, String)>)
            .out_arg(("results", "(ba(os))"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn destroy_filesystems_method_object(
        f: &Factory<MTFn<TData>, TData>,
    ) -> Method<MTFn<TData>, TData> {
        f.method("DestroyFilesystems", (), destroy_filesystems)
            .in_arg(("filesystems", "ao"))
            // b: true if filesystems were destroyed
            // as: Array of UUIDs of destroyed filesystems
            //
            // Rust representation: (bool, Vec<String>)
            .out_arg(("results", "(bas)"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn add_blockdevs_method_object(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
        f.method("AddDataDevs", (), add_datadevs)
            .in_arg(("devices", "as"))
            // b: Indicates if any data devices were added
            // ao: Array of object paths of created data devices
            //
            // Rust representation: (bool, Vec<dbus::path>)
            .out_arg(("results", "(bao)"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn rename_method_object(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
        f.method("SetName", (), rename_pool)
            .in_arg(("name", "s"))
            // b: false if no pool was renamed
            // s: UUID of renamed pool
            //
            // Rust representation: (bool, String)
            .out_arg(("result", "(bs)"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn snapshot_method_object(f: &Factory<MTFn<TData>, TData>) -> Method<MTFn<TData>, TData> {
        f.method("SnapshotFilesystem", (), snapshot_filesystem)
            .in_arg(("origin", "o"))
            .in_arg(("snapshot_name", "s"))
            // b: false if no new snapshot was created
            // s: Object path of new snapshot
            //
            // Rust representation: (bool, String)
            .out_arg(("result", "(bo)"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"))
    }

    fn name_property_object(f: &Factory<MTFn<TData>, TData>) -> Property<MTFn<TData>, TData> {
        f.property::<&str, _>(consts::POOL_NAME_PROP, ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::True)
            .on_get(get_pool_name)
    }

    fn uuid_property_object(f: &Factory<MTFn<TData>, TData>) -> Property<MTFn<TData>, TData> {
        f.property::<&str, _>("Uuid", ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_uuid)
    }

    let get_all_properties_method = f
        .method("GetAllProperties", (), get_all_properties)
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"));

    let get_properties_method = f
        .method("GetProperties", (), get_properties)
        .in_arg(("properties", "as"))
        // a{s(bv)}: Dictionary of property names to tuples
        // In the tuple:
        // b: Indicates whether the property value fetched was successful
        // v: If b is true, represents the value for the given property
        //    If b is false, represents the error returned when fetching the property
        .out_arg(("results", "a{s(bv)}"));

    let object_name = make_object_path(dbus_context);

    let object_path = f
        .object_path(object_name, Some(OPContext::new(parent, uuid)))
        .introspectable()
        .add(
            f.interface(consts::POOL_INTERFACE_NAME, ())
                .add_m(create_filesystems_method_object(&f))
                .add_m(destroy_filesystems_method_object(&f))
                .add_m(snapshot_method_object(&f))
                .add_m(add_blockdevs_method_object(&f))
                .add_m(add_cachedevs_method)
                .add_m(rename_method_object(&f))
                .add_p(name_property_object(&f))
                .add_p(uuid_property_object(&f)),
        )
        .add(
            f.interface(consts::POOL_INTERFACE_NAME_2_1, ())
                .add_m(create_filesystems_method_object(&f))
                .add_m(destroy_filesystems_method_object(&f))
                .add_m(snapshot_method_object(&f))
                .add_m(add_blockdevs_method_object(&f))
                .add_m(init_cache_method_2_1)
                .add_m(add_cachedevs_method_2_1)
                .add_m(rename_method_object(&f))
                .add_p(name_property_object(&f))
                .add_p(uuid_property_object(&f)),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(get_all_properties_method)
                .add_m(get_properties_method),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    pool.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}
