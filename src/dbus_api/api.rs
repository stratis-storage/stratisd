// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Cow;
use std::convert::From;
use std::fmt::Display;
use std::path::Path;
use std::vec::Vec;
use std::error::Error;

use dbus;
use dbus::Connection;
use dbus::BusType;
use dbus::Message;
use dbus::MessageItem;
use dbus::NameFlag;
use dbus::arg::ArgType;
use dbus::arg::Array;
use dbus::arg::Iter;
use dbus::arg::IterAppend;
use dbus::tree::Access;
use dbus::tree::EmitsChangedSignal;
use dbus::tree::Factory;
use dbus::tree::MethodErr;
use dbus::tree::MTFn;
use dbus::tree::MethodResult;
use dbus::tree::MethodInfo;
use dbus::tree::PropInfo;
use dbus::tree::Tree;
use dbus::ConnectionItem;

use super::super::types::Bytes;
use super::super::stratis::VERSION;

use engine;
use engine::Engine;
use engine::EngineError;
use engine::Redundancy;
use engine::RenameAction;

use types::StratisResult;

use super::types::{DeferredAction, DbusContext, DbusErrorEnum, TData};

const STRATIS_BASE_PATH: &'static str = "/org/storage/stratis1";
const STRATIS_BASE_SERVICE: &'static str = "org.storage.stratis1";

/// Convert a tuple as option to an Option type
fn tuple_to_option<T>(value: (bool, T)) -> Option<T> {
    if value.0 { Some(value.1) } else { None }
}

/// Get the next argument off the bus
fn get_next_arg<'a, T>(iter: &mut Iter<'a>, loc: u16) -> Result<T, MethodErr>
    where T: dbus::arg::Get<'a> + dbus::arg::Arg
{
    if iter.arg_type() == ArgType::Invalid {
        return Err(MethodErr::no_arg());
    };
    let value: T = try!(iter.read::<T>().map_err(|_| MethodErr::invalid_arg(&loc)));
    Ok(value)
}

/// Get filesystem name from object path
fn object_path_to_pair(dbus_context: &DbusContext,
                       fs_object_path: &str)
                       -> Result<(String, String), (MessageItem, MessageItem)> {
    let fs_pool_pair = match dbus_context.filesystems.borrow().get_by_first(fs_object_path) {
        Some(fs_name) => fs_name.clone(),
        None => {
            let items = code_to_message_items(DbusErrorEnum::FILESYSTEM_NOTFOUND,
                                              format!("no filesystem for object path {}",
                                                      fs_object_path));
            return Err(items);
        }
    };

    Ok(fs_pool_pair)
}

/// Get name for pool from object path
fn object_path_to_pool_name(dbus_context: &DbusContext,
                            path: &dbus::Path)
                            -> Result<String, (MessageItem, MessageItem)> {
    let pool_name = match dbus_context.pools.borrow().get(path) {
        Some(pool) => pool.clone(),
        None => {
            let items = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR,
                                              format!("no pool for object path {}", path));
            return Err(items);
        }
    };
    Ok(pool_name)
}

/// Macro for early return with Ok dbus message on failure to get pool.
macro_rules! get_pool {
    ( $engine:ident; $name:ident; $default:expr; $message:expr ) => {
        match $engine.get_pool(&$name) {
            Some(pool) => pool,
            None => {
                let (rc, rs) = code_to_message_items(DbusErrorEnum::POOL_NOTFOUND,
                                                     format!("no pool for name {}", $name));
                return Ok(vec![$message.append3($default, rc, rs)]);
            }
        }
    }
}
/// Macros for early return with an Ok dbus message on a dbus error
macro_rules! dbus_try {
    ( $val:expr; $default:expr; $message:expr ) => {
        match $val {
            Ok(v) => v,
            Err((rc, rs)) => {
                return Ok(vec![$message.append3($default, rc, rs)]);
            }
        };
    }
}

macro_rules! dbus_try_0 {
    ( $val:expr; $message:expr ) => {
        match $val {
            Ok(v) => v,
            Err((rc, rs)) => {
                return Ok(vec![$message.append2(rc, rs)]);
            }
        };
    }
}

/// Macros for early return with an Ok dbus message on an engine error
macro_rules! engine_try {
    ( $val:expr; $default:expr; $message:expr ) => {
        match $val {
            Ok(result) => result,
            Err(x) => {
                let (rc, rs) = engine_to_dbus_err(&x);
                let (rc, rs) = code_to_message_items(rc, rs);
                return Ok(vec![$message.append3($default, rc, rs)]);
            }
        }
    }
}

macro_rules! engine_try_0 {
    ( $val:expr; $message:expr ) => {
        match $val {
            Ok(result) => result,
            Err(x) => {
                let (rc, rs) = engine_to_dbus_err(&x);
                let (rc, rs) = code_to_message_items(rc, rs);
                return Ok(vec![$message.append2(rc, rs)]);
            }
        }
    }
}

/// Translates an engine error to a dbus error.
fn engine_to_dbus_err(err: &EngineError) -> (DbusErrorEnum, String) {
    let error = match *err {
        EngineError::Engine(ref e, _) => {
            match *e {
                engine::ErrorEnum::Error => DbusErrorEnum::ERROR,
                engine::ErrorEnum::AlreadyExists => DbusErrorEnum::ALREADY_EXISTS,
                engine::ErrorEnum::Busy => DbusErrorEnum::BUSY,
                engine::ErrorEnum::Invalid => DbusErrorEnum::ERROR,
                engine::ErrorEnum::NotFound => DbusErrorEnum::NOTFOUND,
            }
        }
        EngineError::Io(_) => DbusErrorEnum::IO_ERROR,
        EngineError::Nix(_) => DbusErrorEnum::NIX_ERROR,
        EngineError::Uuid(_) => DbusErrorEnum::INTERNAL_ERROR,
        EngineError::Utf8(_) => DbusErrorEnum::INTERNAL_ERROR,
        EngineError::Serde(_) => DbusErrorEnum::INTERNAL_ERROR,
    };
    (error, err.description().to_owned())
}

/// Convenience function to convert a return code and a string to
/// appropriately typed MessageItems.
fn code_to_message_items(code: DbusErrorEnum, mes: String) -> (MessageItem, MessageItem) {
    (MessageItem::UInt16(code.into()), MessageItem::Str(mes))
}

/// Convenience function to directly yield MessageItems for OK code and message.
fn ok_message_items() -> (MessageItem, MessageItem) {
    let code = DbusErrorEnum::OK;
    code_to_message_items(code, code.get_error_string().into())
}

fn default_object_path<'a>() -> dbus::Path<'a> {
    dbus::Path::new("/").unwrap()
}

fn create_dbus_filesystem<'a>(dbus_context: &DbusContext) -> dbus::Path<'a> {
    let f = Factory::new_fn();

    let create_snapshot_method = f.method("CreateSnapshot", (), create_snapshot)
        .in_arg(("name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method("SetName", (), rename_filesystem)
        .in_arg(("name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_mountpoint_method = f.method("SetMountpoint", (), set_filesystem_mountpoint)
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_quota_method = f.method("SetQuota", (), set_filesystem_quota)
        .in_arg(("quota", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));


    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "filesystem");

    let object_path = f.object_path(object_name, ())
        .introspectable()
        .add(f.interface(interface_name, ())
            .add_m(create_snapshot_method)
            .add_m(rename_method)
            .add_m(set_mountpoint_method)
            .add_m(set_quota_method));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}

fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<(&str, &str, (bool, u64)), _> = try!(get_next_arg(&mut iter, 0));
    let dbus_context = m.tree.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(os)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let specs = filesystems.map(|x| (x.0, x.1, tuple_to_option(x.2).map(|x| Bytes(x))))
        .collect::<Vec<(&str, &str, Option<Bytes>)>>();
    let result = pool.create_filesystems(&specs);

    let msg = match result {
        Ok(ref names) => {
            let mut return_value = Vec::new();
            for name in names {
                let fs_object_path: dbus::Path = create_dbus_filesystem(dbus_context);
                dbus_context.filesystems
                    .borrow_mut()
                    .insert(fs_object_path.to_string(),
                            (pool_name.clone(), (*name).into()));
                return_value.push((fs_object_path, name));
            }

            let return_value = return_value.iter()
                .map(|x| {
                    MessageItem::Struct(vec![MessageItem::ObjectPath(x.0.clone()),
                                             MessageItem::Str((*x.1).into())])
                })
                .collect();
            let return_value = MessageItem::Array(return_value, return_sig.into());
            let (rc, rs) = ok_message_items();
            return_message.append3(return_value, rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])

}

fn create_snapshot(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let snapshot_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::ObjectPath(default_object_path());

    let (pool_name, filesystem_name) = dbus_try!(object_path_to_pair(dbus_context, object_path);
		            default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let msg = match pool.create_snapshot(snapshot_name, &filesystem_name) {
        Ok(_) => {
            let object_path: dbus::Path = create_dbus_filesystem(dbus_context);
            dbus_context.filesystems.borrow_mut().insert(object_path.to_string(),
                                                         ((&pool_name).clone(),
                                                          String::from(snapshot_name)));
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::ObjectPath(object_path), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn set_filesystem_quota(_m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    unimplemented!()
}

fn set_filesystem_mountpoint(_m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    unimplemented!()
}

fn rename_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::Bool(false);

    let (pool_name, filesystem_name) = dbus_try!(
        object_path_to_pair(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let result = pool.rename_filesystem(&filesystem_name, &new_name);

    let msg = match result {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about filesystem {} on pool {}",
                                        filesystem_name,
                                        pool_name);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Renamed) => {
            let return_value = MessageItem::Bool(true);
            let removed = dbus_context.filesystems
                .borrow_mut()
                .remove_by_second(&(pool_name.clone(), filesystem_name.into()));
            match removed {
                Some((removed_object_path, _)) => {
                    if object_path.to_string() == removed_object_path {
                        dbus_context.filesystems
                            .borrow_mut()
                            .insert(removed_object_path, (pool_name.into(), new_name.into()));
                        let (rc, rs) = ok_message_items();
                        return_message.append3(return_value, rc, rs)
                    } else {
                        let error_message = format!("wrong dbus object_path for renamed \
                                                    filesystem");
                        let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR,
                                                             error_message);
                        return_message.append3(return_value, rc, rs)
                    }
                }
                None => {
                    let error_message = format!("no dbus object path for renamed filesystem");
                    let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR,
                                                         error_message);
                    return_message.append3(return_value, rc, rs)
                }
            }
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<&str, _> = try!(get_next_arg(&mut iter, 0));

    let mut filesystem_names = Vec::new();
    for name in filesystems {
        filesystem_names.push(name);
    }

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let result = pool.destroy_filesystems(&filesystem_names);
    let msg = match result {
        Ok(ref names) => {
            for name in names {
                match dbus_context.filesystems
                    .borrow_mut()
                    .remove_by_second(&(pool_name.clone(), (*name).into())) {
                    Some((object_path, _)) => {
                        dbus_context.actions.borrow_mut().push_remove(object_path);
                    }
                    _ => {}
                }
            }

            let return_value = names.iter().map(|n| MessageItem::Str((*n).into())).collect();
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Array(return_value, return_sig.into()), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn add_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let force: bool = try!(get_next_arg(&mut iter, 0));
    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 1));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return;
        return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();
    let result = pool.add_blockdevs(&blockdevs, force);

    let msg = match result {
        Ok(devnodes) => {
            let paths = devnodes.iter().map(|d| d.to_str().unwrap().into());
            let paths = paths.map(|x| MessageItem::Str(x)).collect();
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Array(paths, return_sig.into()), rc, rs)
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}


fn add_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let force: bool = try!(get_next_arg(&mut iter, 0));
    let cache_devs: Array<&str, _> = try!(get_next_arg(&mut iter, 1));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return;
        return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = get_pool!(b_engine; pool_name; default_return; return_message);

    let cachedevs = cache_devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();
    let result = pool.add_cachedevs(&cachedevs, force);

    let msg = match result {
        Ok(devnodes) => {
            let paths = devnodes.iter().map(|d| d.to_str().unwrap().into());
            let paths = paths.map(|x| MessageItem::Str(x)).collect();
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Array(paths, return_sig.into()), rc, rs)
        }
        Err(x) => {
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn rename_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::Bool(false);

    let old_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut engine = dbus_context.engine.borrow_mut();
    let result = engine.rename_pool(&old_name, new_name);

    let msg = match result {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about pool {}", old_name);
            let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(false), rc, rs)
        }
        Ok(RenameAction::Renamed) => {
            let return_value = MessageItem::Bool(true);
            if let Some(name) = dbus_context.pools.borrow_mut().get_mut(&object_path) {
                *name = new_name.into();
                let (rc, rs) = ok_message_items();
                return_message.append3(return_value, rc, rs)
            } else {
                let error_message = format!("wrong dbus object_path for renamed pool");
                let (rc, rs) = code_to_message_items(DbusErrorEnum::INTERNAL_ERROR, error_message);
                return_message.append3(return_value, rc, rs)
            }
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn create_dbus_pool<'a>(dbus_context: &DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_filesystems_method = f.method("CreateFilesystems", (), create_filesystems)
        .in_arg(("filesystems", "a(ss(bt))"))
        .out_arg(("filesystems", "a(os)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_filesystems_method = f.method("DestroyFilesystems", (), destroy_filesystems)
        .in_arg(("filesystems", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_cache_devs_method = f.method("AddCacheDevs", (), add_cache_devs)
        .in_arg(("force", "b"))
        .in_arg(("cache_devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_devs_method = f.method("AddDevs", (), add_devs)
        .in_arg(("force", "b"))
        .in_arg(("devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method("SetName", (), rename_pool)
        .in_arg(("new_name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "pool");

    let object_path = f.object_path(object_name, ())
        .introspectable()
        .add(f.interface(interface_name, ())
            .add_m(create_filesystems_method)
            .add_m(destroy_filesystems_method)
            .add_m(add_cache_devs_method)
            .add_m(add_devs_method)
            .add_m(rename_method));

    let path = object_path.get_name().to_owned();
    dbus_context.actions.borrow_mut().push_add(object_path);
    path
}

fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = try!(get_next_arg(&mut iter, 0));
    let redundancy: (bool, u16) = try!(get_next_arg(&mut iter, 1));
    let force: bool = try!(get_next_arg(&mut iter, 2));
    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 3));

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let dbus_context = m.tree.get_data();
    let result = dbus_context.engine
        .borrow_mut()
        .create_pool(name, &blockdevs, tuple_to_option(redundancy), force);

    let return_message = message.method_return();

    let msg = match result {
        Ok(devnodes) => {
            let object_path: dbus::Path = create_dbus_pool(dbus_context);
            dbus_context.pools.borrow_mut().insert(object_path.clone(), String::from(name));
            let paths = devnodes.iter().map(|d| d.to_str().unwrap().into());
            let paths = paths.map(|x| MessageItem::Str(x)).collect();
            let return_path = MessageItem::ObjectPath(object_path);
            let return_list = MessageItem::Array(paths, "s".into());
            let return_value = MessageItem::Struct(vec![return_path, return_list]);
            let (rc, rs) = ok_message_items();
            return_message.append3(return_value, rc, rs)
        }
        Err(x) => {
            let return_path = MessageItem::ObjectPath(default_object_path());
            let return_list = MessageItem::Array(vec![], "s".into());
            let return_value = MessageItem::Struct(vec![return_path, return_list]);
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(return_value, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let object_path: dbus::Path<'static> = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let ref engine = dbus_context.engine;

    let default_return = MessageItem::Bool(false);
    let return_message = message.method_return();

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, &object_path);
        default_return; return_message);

    let result = engine.borrow_mut().destroy_pool(&pool_name);

    let msg = match result {
        Ok(action) => {
            dbus_context.pools.borrow_mut().remove(&object_path);
            dbus_context.actions
                .borrow_mut()
                .push_remove(object_path.as_cstr().to_str().unwrap().into());
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(action), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(default_return, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_list_items<T, I>(i: &mut IterAppend, iter: I) -> Result<(), MethodErr>
    where T: Display + Into<u16>,
          I: Iterator<Item = T>
{
    let msg_vec = iter.map(|item| {
            MessageItem::Struct(vec![MessageItem::Str(format!("{}", item)),
                                     MessageItem::UInt16(item.into())])
        })
        .collect::<Vec<MessageItem>>();
    let item_array = MessageItem::Array(msg_vec, Cow::Borrowed("(sq)"));
    i.append(item_array);
    Ok(())
}

fn get_error_values(i: &mut IterAppend,
                    _p: &PropInfo<MTFn<TData>, TData>)
                    -> Result<(), MethodErr> {
    get_list_items(i, DbusErrorEnum::iter_variants())
}


fn get_redundancy_values(i: &mut IterAppend,
                         _p: &PropInfo<MTFn<TData>, TData>)
                         -> Result<(), MethodErr> {
    get_list_items(i, Redundancy::iter_variants())
}

fn get_version(i: &mut IterAppend, _p: &PropInfo<MTFn<TData>, TData>) -> Result<(), MethodErr> {
    i.append(VERSION);
    Ok(())
}

fn configure_simulator(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let denominator: u32 = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.tree.get_data();
    let result = dbus_context.engine.borrow_mut().configure_simulator(denominator);

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => {
            let (rc, rs) = ok_message_items();
            return_message.append2(rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append2(rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_base_tree<'a>(dbus_context: DbusContext) -> Tree<MTFn<TData>, TData> {

    let f = Factory::new_fn();

    let base_tree = f.tree(dbus_context);

    let create_pool_method = f.method("CreatePool", (), create_pool)
        .in_arg(("pool_name", "s"))
        .in_arg(("redundancy", "(bq)"))
        .in_arg(("force", "b"))
        .in_arg(("dev_list", "as"))
        .out_arg(("result", "(oas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_pool_method = f.method("DestroyPool", (), destroy_pool)
        .in_arg(("pool_object_path", "o"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let configure_simulator_method = f.method("ConfigureSimulator", (), configure_simulator)
        .in_arg(("denominator", "u"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let redundancy_values_property =
        f.property::<Array<(&str, u16), &Iterator<Item = (&str, u16)>>, _>("RedundancyValues",
                                                                              ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_redundancy_values);

    let error_values_property =
        f.property::<Array<(&str, u16), &Iterator<Item = (&str, u16)>>, _>("ErrorValues", ())
            .access(Access::Read)
            .emits_changed(EmitsChangedSignal::Const)
            .on_get(get_error_values);

    let version_property = f.property::<&str, _>("Version", ())
        .access(Access::Read)
        .emits_changed(EmitsChangedSignal::Const)
        .on_get(get_version);

    let interface_name = format!("{}.{}", STRATIS_BASE_SERVICE, "Manager");

    let obj_path = f.object_path(STRATIS_BASE_PATH, ())
        .introspectable()
        .object_manager()
        .add(f.interface(interface_name, ())
            .add_m(create_pool_method)
            .add_m(destroy_pool_method)
            .add_m(configure_simulator_method)
            .add_p(error_values_property)
            .add_p(redundancy_values_property)
            .add_p(version_property));

    base_tree.add(obj_path)
}

pub fn run(engine: Box<Engine>) -> StratisResult<()> {
    let c = try!(Connection::get_private(BusType::Session));

    let mut tree = get_base_tree(DbusContext::new(engine));
    let dbus_context = tree.get_data().clone();
    try!(tree.set_registered(&c, true));

    c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32).unwrap();

    // ...and serve incoming requests.
    for c_item in c.iter(10000) {
        if let ConnectionItem::MethodCall(ref msg) = c_item {
            if let Some(v) = tree.handle(&msg) {
                // Probably the wisest is to ignore any send errors here -
                // maybe the remote has disconnected during our processing.
                for m in v {
                    let _ = c.send(m);
                }
            }
            let mut b_actions = dbus_context.actions.borrow_mut();
            for action in b_actions.drain() {
                match action {
                    DeferredAction::Add(path) => {
                        try!(c.register_object_path(path.get_name()));
                        tree.insert(path);
                    }
                    DeferredAction::Remove(path) => {
                        c.unregister_object_path(&path);
                        tree.remove(&dbus::Path::new(path).unwrap());
                    }
                }
            }
        }
    }
    Ok(())
}
