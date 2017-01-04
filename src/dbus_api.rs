// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use bidir_map::BidirMap;

use std::borrow::Cow;
use std::cell::Cell;
use std::cell::RefCell;
use std::fmt::Display;
use std::path::Path;
use std::rc::Rc;
use std::vec::Vec;

use dbus;
use dbus::Connection;
use dbus::BusType;
use dbus::Message;
use dbus::MessageItem;
use dbus::NameFlag;
use dbus::arg::Array;
use dbus::arg::Iter;
use dbus::tree::Factory;
use dbus::tree::DataType;
use dbus::tree::MethodErr;
use dbus::tree::MTFn;
use dbus::tree::MethodResult;
use dbus::tree::MethodInfo;
use dbus::tree::Tree;
use dbus::tree::ObjectPath;
use dbus::ConnectionItem;

use dbus_consts::*;

use engine;
use engine::Engine;
use engine::EngineError;
use engine::RenameAction;

use types::StratisResult;

#[derive(Debug)]
pub enum DeferredAction {
    Add(ObjectPath<MTFn<TData>, TData>),
    Remove(String),
}

#[derive(Debug, Clone)]
pub struct DbusContext {
    pub next_index: Rc<Cell<u64>>,
    pub pools: Rc<RefCell<BidirMap<String, String>>>,
    pub engine: Rc<RefCell<Box<Engine>>>,
    pub action_list: Rc<RefCell<Vec<DeferredAction>>>,
    pub filesystems: Rc<RefCell<BidirMap<String, (String, String)>>>,
}

impl DbusContext {
    pub fn new(engine: Rc<RefCell<Box<Engine>>>) -> DbusContext {
        DbusContext {
            action_list: Rc::new(RefCell::new(Vec::new())),
            engine: engine.clone(),
            filesystems: Rc::new(RefCell::new(BidirMap::new())),
            next_index: Rc::new(Cell::new(0)),
            pools: Rc::new(RefCell::new(BidirMap::new())),
        }
    }

    /// Generates a new id for object paths.
    /// It is assumed that, while Stratisd is running, it will never generate
    /// more than 2^64 object paths. If it turns out that this is a bad
    /// assumption, the solution is to use unbounded integers.
    pub fn get_next_id(&mut self) -> u64 {
        self.next_index.set(self.next_index.get() + 1);
        self.next_index.get()
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct TData;
impl DataType for TData {
    type ObjectPath = DbusContext;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}

/// Convert a tuple as option to an Option type
fn tuple_to_option<T>(value: (bool, T)) -> Option<T> {
    if value.0 { Some(value.1) } else { None }
}

/// Get the next argument off the bus
fn get_next_arg<'a, T>(iter: &mut Iter<'a>, loc: u16) -> Result<T, MethodErr>
    where T: dbus::arg::Get<'a> + dbus::arg::Arg
{
    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    };
    let value: T = try!(iter.read::<T>().map_err(|_| MethodErr::invalid_arg(&loc)));
    Ok(value)
}

/// Get object path from filesystem name
fn fs_name_to_object_path(dbus_context: &DbusContext,
                          pool_name: &str,
                          name: &str)
                          -> Result<String, (MessageItem, MessageItem)> {
    let object_path =
        match dbus_context.filesystems.borrow().get_by_second(&(pool_name.into(), name.into())) {
            Some(pool) => pool.clone(),
            None => {
                let items = code_to_message_items(ErrorEnum::FILESYSTEM_NOTFOUND,
                                                  format!("no object path for filesystem {} \
                                                           belonging to pool {}",
                                                          name,
                                                          pool_name));
                return Err(items);
            }
        };
    Ok(object_path)
}

/// Get filesystem name from object path
fn object_path_to_pair(dbus_context: &DbusContext,
                       fs_object_path: &str)
                       -> Result<(String, String), (MessageItem, MessageItem)> {
    let fs_pool_pair = match dbus_context.filesystems.borrow().get_by_first(fs_object_path) {
        Some(fs_name) => fs_name.clone(),
        None => {
            let items = code_to_message_items(ErrorEnum::FILESYSTEM_NOTFOUND,
                                              format!("no filesystem for object path {}",
                                                      fs_object_path));
            return Err(items);
        }
    };

    Ok(fs_pool_pair)
}

/// Get object path from pool name
fn pool_name_to_object_path(dbus_context: &DbusContext,
                            name: &str)
                            -> Result<String, (MessageItem, MessageItem)> {
    let object_path = match dbus_context.pools.borrow().get_by_second(name) {
        Some(pool) => pool.clone(),
        None => {
            let items = code_to_message_items(ErrorEnum::POOL_NOTFOUND,
                                              format!("no object path for pool name {}", name));
            return Err(items);
        }
    };
    Ok(object_path)
}

/// Convert a string from a object path/name map to an object path
fn string_to_object_path<'a>(path: String) -> Result<dbus::Path<'a>, (MessageItem, MessageItem)> {
    let object_path = match dbus::Path::new(path) {
        Ok(p) => p,
        Err(s) => {
            let items = code_to_message_items(ErrorEnum::INTERNAL_ERROR,
                                              format!("malformed object path {} in table", s));
            return Err(items);
        }
    };
    Ok(object_path)
}

/// Get name for pool from object path
fn object_path_to_pool_name(dbus_context: &DbusContext,
                            path: &str)
                            -> Result<String, (MessageItem, MessageItem)> {
    let pool_name = match dbus_context.pools.borrow().get_by_first(path) {
        Some(pool) => pool.clone(),
        None => {
            let items = code_to_message_items(ErrorEnum::INTERNAL_ERROR,
                                              format!("no pool for object path {}", path));
            return Err(items);
        }
    };
    Ok(pool_name)
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

/// Translates an engine ErrorEnum to a dbus ErrorEnum.
fn engine_to_dbus_enum(err: &engine::ErrorEnum) -> (ErrorEnum, String) {
    match *err {
        engine::ErrorEnum::Ok => (ErrorEnum::OK, err.get_error_string()),
        engine::ErrorEnum::Error(_) => (ErrorEnum::ERROR, err.get_error_string()),
        engine::ErrorEnum::AlreadyExists(_) => (ErrorEnum::ALREADY_EXISTS, err.get_error_string()),
        engine::ErrorEnum::Busy(_) => (ErrorEnum::BUSY, err.get_error_string()),
        engine::ErrorEnum::Invalid(_) => (ErrorEnum::ERROR, err.get_error_string()),
        engine::ErrorEnum::NotFound(_) => (ErrorEnum::NOTFOUND, err.get_error_string()),
    }
}

/// Translates an engine error to a dbus error.
fn engine_to_dbus_err(err: &EngineError) -> (ErrorEnum, String) {
    match *err {
        EngineError::Stratis(ref e) => engine_to_dbus_enum(e),
        EngineError::Io(_) => {
            let error = ErrorEnum::IO_ERROR;
            (error, error.get_error_string().into())
        }
        EngineError::Nix(_) => {
            let error = ErrorEnum::NIX_ERROR;
            (error, error.get_error_string().into())
        }
    }
}

/// Convenience function to convert a return code and a string to
/// appropriately typed MessageItems.
fn code_to_message_items(code: ErrorEnum, mes: String) -> (MessageItem, MessageItem) {
    (MessageItem::UInt16(code.get_error_int()), MessageItem::Str(mes))
}

/// Convenience function to directly yield MessageItems for OK code and message.
fn ok_message_items() -> (MessageItem, MessageItem) {
    let code = ErrorEnum::OK;
    code_to_message_items(code, code.get_error_string().into())
}

fn default_object_path<'a>() -> dbus::Path<'a> {
    dbus::Path::new(DEFAULT_OBJECT_PATH).unwrap()
}

fn remove_dbus_object_path(dbus_context: &DbusContext, path: String) {
    dbus_context.action_list.borrow_mut().push(DeferredAction::Remove(path));
}

fn list_pools(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let dbus_context = m.path.get_data();
    let mut engine = dbus_context.engine.borrow_mut();

    let result = engine.pools();
    let msg_vec = result.keys().map(|key| MessageItem::Str((*key).into())).collect();
    let item_array = MessageItem::Array(msg_vec, "s".into());
    let (rc, rs) = ok_message_items();
    let msg = m.msg.method_return().append3(item_array, rc, rs);
    Ok(vec![msg])
}

fn create_dbus_filesystem<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_snapshot_method = f.method(CREATE_SNAPSHOT, (), create_snapshot)
        .in_arg(("name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method(RENAME_FILESYSTEM, (), rename_filesystem)
        .in_arg(("name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_mountpoint_method = f.method(SET_MOUNTPOINT, (), set_filesystem_mountpoint)
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_quota_method = f.method(SET_QUOTA, (), set_filesystem_quota)
        .in_arg(("quota", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));


    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let object_path = f.object_path(object_name, dbus_context.clone())
        .introspectable()
        .add(f.interface(STRATIS_FILESYSTEM_BASE_INTERFACE, ())
            .add_m(create_snapshot_method)
            .add_m(rename_method)
            .add_m(set_mountpoint_method)
            .add_m(set_quota_method));

    let path = object_path.get_name().to_owned();
    dbus_context.action_list.borrow_mut().push(DeferredAction::Add(object_path));
    path
}

fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let filesystems: Array<(&str, &str, (bool, u64)), _> = try!(get_next_arg(&mut iter, 0));
    let dbus_context = m.path.get_data();

    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(os)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let specs = filesystems.map(|x| (x.0, x.1, tuple_to_option(x.2)))
        .collect::<Vec<(&str, &str, Option<u64>)>>();
    let result = pool.create_filesystems(&specs);

    let msg = match result {
        Ok(ref names) => {
            let mut return_value = Vec::new();
            for name in names {
                let fs_object_path: dbus::Path = create_dbus_filesystem(dbus_context.clone());
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

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::ObjectPath(default_object_path());

    let (pool_name, filesystem_name) = dbus_try!(object_path_to_pair(dbus_context, object_path);
		            default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let pool = engine_try!(b_engine.get_pool(&pool_name);default_return; return_message);

    let msg = match pool.create_snapshot(snapshot_name, &filesystem_name) {
        Ok(_) => {
            let object_path: dbus::Path = create_dbus_filesystem(dbus_context.clone());
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

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let default_return = MessageItem::Bool(false);

    let (pool_name, filesystem_name) = dbus_try!(
        object_path_to_pair(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(
        b_engine.get_pool(&pool_name);
        default_return; return_message);

    let result = pool.rename_filesystem(&filesystem_name, &new_name);

    let msg = match result {
        Ok(RenameAction::NoSource) => {
            let error_message = format!("engine doesn't know about filesystem {} on pool {}",
                                        filesystem_name,
                                        pool_name);
            let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR, error_message);
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
                        let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR,
                                                             error_message);
                        return_message.append3(return_value, rc, rs)
                    }
                }
                None => {
                    let error_message = format!("no dbus object path for renamed filesystem");
                    let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR, error_message);
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

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.destroy_filesystems(&filesystem_names);
    let msg = match result {
        Ok(ref names) => {
            for name in names {
                match dbus_context.filesystems
                    .borrow_mut()
                    .remove_by_second(&(pool_name.clone(), (*name).into())) {
                    Some((object_path, _)) => {
                        remove_dbus_object_path(dbus_context, object_path);
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

fn list_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.filesystems();
    let msg_vec = result.keys().map(|key| MessageItem::Str((*key).into())).collect();
    let item_array = MessageItem::Array(msg_vec, return_sig.into());
    let (rc, rs) = ok_message_items();
    let msg = return_message.append3(item_array, rc, rs);
    Ok(vec![msg])
}

fn list_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.blockdevs();
    let msg_vec = result.iter().map(|x| MessageItem::Str(x.get_id().into())).collect();
    let item_array = MessageItem::Array(msg_vec, return_sig.into());
    let (rc, rs) = ok_message_items();
    let msg = return_message.append3(item_array, rc, rs);
    Ok(vec![msg])
}

fn list_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.cachedevs();
    let msg_vec = result.iter().map(|x| MessageItem::Str(x.get_id().into())).collect();
    let item_array = MessageItem::Array(msg_vec, return_sig.into());
    let (rc, rs) = ok_message_items();
    let msg = return_message.append3(item_array, rc, rs);
    Ok(vec![msg])
}

fn remove_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 0));

    let mut device_paths = Vec::new();
    for path in devs {
        device_paths.push(Path::new(path));
    }

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.remove_cachedevs(&device_paths);

    let msg = match result {
        Ok(ref paths) => {
            let return_value = paths.iter()
                .map(|p| MessageItem::Str((*p).to_string_lossy().into_owned()))
                .collect();
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

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return;
        return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

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

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return;
        return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

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

fn remove_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 0));

    let mut device_paths = Vec::new();
    for path in devs {
        device_paths.push(Path::new(path));
    }

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, object_path);
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.remove_blockdevs(&device_paths);

    let msg = match result {
        Ok(ref paths) => {
            let return_value = paths.iter()
                .map(|p| MessageItem::Str((*p).to_string_lossy().into_owned()))
                .collect();
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

fn rename_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let new_name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.path.get_data();
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
            let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR, error_message);
            return_message.append3(default_return, rc, rs)
        }
        Ok(RenameAction::Identity) => {
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(false), rc, rs)
        }
        Ok(RenameAction::Renamed) => {
            let return_value = MessageItem::Bool(true);
            let removed = dbus_context.pools.borrow_mut().remove_by_second(&old_name);
            match removed {
                Some((removed_object_path, _)) => {
                    if object_path.to_string() == removed_object_path {
                        dbus_context.pools
                            .borrow_mut()
                            .insert(removed_object_path, new_name.into());
                        let (rc, rs) = ok_message_items();
                        return_message.append3(return_value, rc, rs)
                    } else {
                        let error_message = format!("wrong dbus object_path for renamed pool");
                        let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR,
                                                             error_message);
                        return_message.append3(return_value, rc, rs)
                    }
                }
                None => {
                    let error_message = format!("no dbus object path for renamed pool");
                    let (rc, rs) = code_to_message_items(ErrorEnum::INTERNAL_ERROR, error_message);
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

fn create_dbus_pool<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_filesystems_method = f.method(CREATE_FILESYSTEMS, (), create_filesystems)
        .in_arg(("filesystems", "a(ss(bt))"))
        .out_arg(("filesystems", "a(os)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_filesystems_method = f.method(DESTROY_FILESYSTEMS, (), destroy_filesystems)
        .in_arg(("filesystems", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let list_filesystems_method = f.method(LIST_FILESYSTEMS, (), list_filesystems)
        .out_arg(("filesystems", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let list_cache_devs_method = f.method(LIST_CACHE_DEVS, (), list_cache_devs)
        .out_arg(("cache_devs", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_cache_devs_method = f.method(ADD_CACHE_DEVS, (), add_cache_devs)
        .in_arg(("force", "b"))
        .in_arg(("cache_devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let remove_cache_devs_method = f.method(REMOVE_CACHE_DEVS, (), remove_cache_devs)
        .in_arg(("cache_devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let list_devs_method = f.method(LIST_DEVS, (), list_devs)
        .out_arg(("devs", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_devs_method = f.method(ADD_DEVS, (), add_devs)
        .in_arg(("force", "b"))
        .in_arg(("devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let remove_devs_method = f.method(REMOVE_DEVS, (), remove_devs)
        .in_arg(("devs", "as"))
        .out_arg(("results", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method(RENAME_POOL, (), rename_pool)
        .in_arg(("new_name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let object_path = f.object_path(object_name, dbus_context.clone())
        .introspectable()
        .add(f.interface(STRATIS_POOL_BASE_INTERFACE, ())
            .add_m(create_filesystems_method)
            .add_m(destroy_filesystems_method)
            .add_m(list_filesystems_method)
            .add_m(list_devs_method)
            .add_m(list_cache_devs_method)
            .add_m(add_cache_devs_method)
            .add_m(remove_cache_devs_method)
            .add_m(add_devs_method)
            .add_m(remove_devs_method)
            .add_m(rename_method));

    let path = object_path.get_name().to_owned();
    dbus_context.action_list.borrow_mut().push(DeferredAction::Add(object_path));
    path
}

fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = try!(get_next_arg(&mut iter, 0));
    let raid_level: u16 = try!(get_next_arg(&mut iter, 1));
    let force: bool = try!(get_next_arg(&mut iter, 2));
    let devs: Array<&str, _> = try!(get_next_arg(&mut iter, 3));

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let dbus_context = m.path.get_data();
    let result = dbus_context.engine.borrow_mut().create_pool(name, &blockdevs, raid_level, force);

    let return_message = message.method_return();

    let msg = match result {
        Ok(devnodes) => {
            let object_path: dbus::Path = create_dbus_pool(dbus_context.clone());
            dbus_context.pools.borrow_mut().insert(object_path.to_string(), String::from(name));
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

    let name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.engine;
    let result = engine.borrow_mut().destroy_pool(&name);

    let return_message = message.method_return();

    let msg = match result {
        Ok(action) => {
            match dbus_context.pools.borrow_mut().remove_by_second(name.into()) {
                Some((object_path, _)) => {
                    remove_dbus_object_path(dbus_context, object_path);
                }
                _ => {}
            };
            let (rc, rs) = ok_message_items();
            return_message.append3(MessageItem::Bool(action), rc, rs)
        }
        Err(err) => {
            let (rc, rs) = engine_to_dbus_err(&err);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(MessageItem::Bool(false), rc, rs)
        }
    };
    Ok(vec![msg])
}

fn get_pool_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let name: &str = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.path.get_data();
    let return_message = message.method_return();
    let default_return = MessageItem::ObjectPath(default_object_path());
    let result = pool_name_to_object_path(dbus_context, name);
    let object_path = dbus_try!(result; default_return; return_message);
    let path =
        dbus_try!(string_to_object_path(object_path.clone()); default_return; return_message);
    let (rc, rs) = ok_message_items();
    Ok(vec![return_message.append3(MessageItem::ObjectPath(path), rc, rs)])
}

fn get_filesystem_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let pool_name: &str = try!(get_next_arg(&mut iter, 0));
    let name: &str = try!(get_next_arg(&mut iter, 1));

    let dbus_context = m.path.get_data();
    let return_message = message.method_return();
    let default_return = MessageItem::ObjectPath(default_object_path());
    let result = fs_name_to_object_path(dbus_context, pool_name, name);
    let object_path = dbus_try!(result; default_return; return_message);

    let path =
        dbus_try!(string_to_object_path(object_path.clone()); default_return; return_message);
    let (rc, rs) = ok_message_items();
    Ok(vec![return_message.append3(MessageItem::ObjectPath(path), rc, rs)])
}

fn get_list_items<T, I>(m: &MethodInfo<MTFn<TData>, TData>, iter: I) -> MethodResult
    where T: HasCodes + Display,
          I: Iterator<Item = T>
{
    let msg_vec = iter.map(|item| {
            MessageItem::Struct(vec![MessageItem::Str(format!("{}", item)),
                                     MessageItem::UInt16(item.get_error_int()),
                                     MessageItem::Str(format!("{}", item.get_error_string()))])
        })
        .collect::<Vec<MessageItem>>();

    let item_array = MessageItem::Array(msg_vec, Cow::Borrowed("(sqs)"));
    Ok(vec![m.msg.method_return().append1(item_array)])
}

fn get_error_codes(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_list_items(m, ErrorEnum::iter_variants())
}


fn get_raid_levels(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_list_items(m, RaidType::iter_variants())
}

fn configure_simulator(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message = m.msg;
    let mut iter = message.iter_init();

    let denominator: u32 = try!(get_next_arg(&mut iter, 0));

    let dbus_context = m.path.get_data();
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

fn get_base_tree<'a>(dbus_context: DbusContext) -> StratisResult<Tree<MTFn<TData>, TData>> {

    let f = Factory::new_fn();

    let base_tree = f.tree();

    let create_pool_method = f.method(CREATE_POOL, (), create_pool)
        .in_arg(("pool_name", "s"))
        .in_arg(("raid_type", "q"))
        .in_arg(("force", "b"))
        .in_arg(("dev_list", "as"))
        .out_arg(("result", "(oas)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_pool_method = f.method(DESTROY_POOL, (), destroy_pool)
        .in_arg(("pool_name", "s"))
        .out_arg(("action", "b"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let list_pools_method = f.method(LIST_POOLS, (), list_pools)
        .out_arg(("pool_names", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let get_pool_object_path_method = f.method(GET_POOL_OBJECT_PATH, (), get_pool_object_path)
        .in_arg(("pool_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let get_filesystem_object_path_method =
        f.method(GET_FILESYSTEM_OBJECT_PATH, (), get_filesystem_object_path)
            .in_arg(("pool_name", "s"))
            .in_arg(("filesystem_name", "s"))
            .out_arg(("object_path", "o"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"));

    let get_error_codes_method = f.method(GET_ERROR_CODES, (), get_error_codes)
        .out_arg(("error_codes", "a(sqs)"));

    let get_raid_levels_method = f.method(GET_RAID_LEVELS, (), get_raid_levels)
        .out_arg(("error_codes", "a(sqs)"));

    let configure_simulator_method = f.method(CONFIGURE_SIMULATOR, (), configure_simulator)
        .in_arg(("denominator", "u"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let obj_path = f.object_path(STRATIS_BASE_PATH, dbus_context)
        .introspectable()
        .object_manager()
        .add(f.interface(STRATIS_MANAGER_INTERFACE, ())
            .add_m(list_pools_method)
            .add_m(create_pool_method)
            .add_m(destroy_pool_method)
            .add_m(get_pool_object_path_method)
            .add_m(get_filesystem_object_path_method)
            .add_m(get_error_codes_method)
            .add_m(get_raid_levels_method)
            .add_m(configure_simulator_method));

    let base_tree = base_tree.add(obj_path);

    Ok(base_tree)
}

pub fn run(engine: Box<Engine>) -> StratisResult<()> {
    let dbus_context = DbusContext::new(Rc::new(RefCell::new(engine)));
    let mut tree = get_base_tree(dbus_context.clone()).unwrap();

    // Setup DBus connection
    let c = try!(Connection::get_private(BusType::Session));
    c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32).unwrap();
    try!(tree.set_registered(&c, true));

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
            let mut b_action_list = dbus_context.action_list.borrow_mut();
            for action in b_action_list.drain(..) {
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
