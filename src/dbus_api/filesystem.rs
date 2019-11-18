// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use chrono::SecondsFormat;
use dbus::{
    self,
    arg::{Array, IterAppend, RefArg, Variant},
    tree::{
        Access, EmitsChangedSignal, Factory, MTFn, MethodErr, MethodInfo, MethodResult, PropInfo,
        Tree,
    },
    Message, Path,
};
use itertools::Itertools;

use crate::{
    dbus_api::{
        consts,
        types::{DbusContext, DbusErrorEnum, OPContext, TData},
        util::{
            engine_to_dbus_err_tuple, get_next_arg, get_parent, get_uuid, make_object_path,
            msg_code_ok, msg_string_ok,
        },
    },
    engine::{
        filesystem_mount_path, Filesystem, FilesystemUuid, MaybeDbusPath, Name, RenameAction,
    },
};

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
            consts::FILESYSTEM_USED_PROP => Some((
                prop,
                filesystem_operation(m.tree, object_path.get_name(), |(_, _, fs)| {
                    fs.used()
                        .map(|u| (*u).to_string())
                        .map_err(|e| e.to_string())
                }),
            )),
            _ => None,
        })
        .map(|(key, result)| {
            let (success, value) = match result {
                Ok(value) => (true, Variant(Box::new(value) as Box<dyn RefArg>)),
                Err(e) => (false, Variant(Box::new(e) as Box<dyn RefArg>)),
            };
            (key, (success, value))
        })
        .collect();

    Ok(vec![return_message.append1(return_value)])
}

fn get_all_properties(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_properties_shared(
        m,
        &mut vec![consts::FILESYSTEM_USED_PROP]
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

pub fn create_dbus_filesystem<'a>(
    dbus_context: &DbusContext,
    parent: dbus::Path<'static>,
    uuid: FilesystemUuid,
    filesystem: &mut dyn Filesystem,
) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let rename_method = f
        .method("SetName", (), rename_filesystem)
        .in_arg(("name", "s"))
        // b: true if UUID of changed resource has been returned
        // s: UUID of changed resource
        //
        // Rust representation: (bool, String)
        .out_arg(("result", "(bs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let devnode_property = f
        .property::<&str, _>("Devnode", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_filesystem_devnode);

    let name_property = f
        .property::<&str, _>(consts::FILESYSTEM_NAME_PROP, ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::True)
        .on_get(get_filesystem_name);

    let pool_property = f
        .property::<&dbus::Path, _>("Pool", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_parent);

    let uuid_property = f
        .property::<&str, _>("Uuid", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_uuid);

    let created_property = f
        .property::<&str, _>("Created", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_filesystem_created);

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
            f.interface(consts::FILESYSTEM_INTERFACE_NAME, ())
                .add_m(rename_method)
                .add_p(devnode_property)
                .add_p(name_property)
                .add_p(pool_property)
                .add_p(uuid_property)
                .add_p(created_property),
        )
        .add(
            f.interface(consts::PROPERTY_FETCH_INTERFACE_NAME, ())
                .add_m(get_all_properties_method)
                .add_m(get_properties_method),
        );

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    filesystem.set_dbus_path(MaybeDbusPath(Some(path.clone())));
    path
}

fn rename_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = get_next_arg(&mut iter, 0)?;

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = (false, uuid_to_string!(FilesystemUuid::nil()));

    let filesystem_path = m
        .tree
        .get(object_path)
        .expect("implicit argument must be in tree");
    let filesystem_data = get_data!(filesystem_path; default_return; return_message);

    let pool_path = get_parent!(m; filesystem_data; default_return; return_message);
    let pool_uuid = get_data!(pool_path; default_return; return_message).uuid;

    let mut engine = dbus_context.engine.borrow_mut();
    let (pool_name, pool) = get_mut_pool!(engine; pool_uuid; default_return; return_message);

    let msg = match pool.rename_filesystem(&pool_name, filesystem_data.uuid, new_name) {
        Ok(RenameAction::NoSource) => {
            let error_message = format!(
                "pool {} doesn't know about filesystem {}",
                pool_uuid, filesystem_data.uuid
            );
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

/// Get execute a given closure providing a filesystem object and return
/// the calculated value
fn filesystem_operation<F, R>(
    tree: &Tree<MTFn<TData>, TData>,
    object_path: &Path<'static>,
    closure: F,
) -> Result<R, String>
where
    F: Fn((Name, Name, &dyn Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    let dbus_context = tree.get_data();

    let filesystem_path = tree
        .get(object_path)
        .expect("tree must contain implicit argument");

    let filesystem_data = filesystem_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?;

    let pool_path = tree
        .get(&filesystem_data.parent)
        .ok_or_else(|| format!("no path for parent object path {}", &filesystem_data.parent))?;

    let pool_uuid = pool_path
        .get_data()
        .as_ref()
        .ok_or_else(|| format!("no data for object path {}", object_path))?
        .uuid;

    let engine = dbus_context.engine.borrow();
    let (pool_name, pool) = engine
        .get_pool(pool_uuid)
        .ok_or_else(|| format!("no pool corresponding to uuid {}", &pool_uuid))?;
    let filesystem_uuid = filesystem_data.uuid;
    let (fs_name, fs) = pool
        .get_filesystem(filesystem_uuid)
        .ok_or_else(|| format!("no name for filesystem with uuid {}", &filesystem_uuid))?;
    closure((pool_name, fs_name, fs))
}

/// Get a filesystem property and place it on the D-Bus. The property is
/// found by means of the getter method which takes a reference to a
/// Filesystem and obtains the property from the filesystem.
fn get_filesystem_property<F, R>(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
    getter: F,
) -> Result<(), MethodErr>
where
    F: Fn((Name, Name, &dyn Filesystem)) -> Result<R, String>,
    R: dbus::arg::Append,
{
    i.append(
        filesystem_operation(p.tree, p.path.get_name(), getter)
            .map_err(|ref e| MethodErr::failed(e))?,
    );
    Ok(())
}

/// Get the devnode for an object path.
fn get_filesystem_devnode(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(pool_name, fs_name, _)| {
        Ok(format!(
            "{}",
            filesystem_mount_path(pool_name, fs_name).display()
        ))
    })
}

fn get_filesystem_name(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, fs_name, _)| Ok(fs_name.to_owned()))
}

/// Get the creation date and time in rfc3339 format.
fn get_filesystem_created(
    i: &mut IterAppend,
    p: &PropInfo<MTFn<TData>, TData>,
) -> Result<(), MethodErr> {
    get_filesystem_property(i, p, |(_, _, fs)| {
        Ok(fs.created().to_rfc3339_opts(SecondsFormat::Secs, true))
    })
}
