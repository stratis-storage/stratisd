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

use types::StratisResult;

#[derive(Debug)]
pub enum DeferredAction {
    Add(ObjectPath<MTFn<TData>, TData>),
    Remove(ObjectPath<MTFn<TData>, TData>),
}

#[derive(Debug, Clone)]
pub struct DbusContext {
    pub next_index: Rc<Cell<u64>>,
    pub pools: Rc<RefCell<BidirMap<String, String>>>,
    pub engine: Rc<RefCell<Engine>>,
    pub action_list: Rc<RefCell<Vec<DeferredAction>>>,
    pub filesystems: Rc<RefCell<BidirMap<String, (String, String)>>>,
    pub block_devs: Rc<RefCell<BidirMap<String, (String, String)>>>,
    pub cache_devs: Rc<RefCell<BidirMap<String, (String, String)>>>,
}

impl DbusContext {
    pub fn new(engine: &Rc<RefCell<Engine>>) -> DbusContext {
        DbusContext {
            action_list: Rc::new(RefCell::new(Vec::new())),
            engine: engine.clone(),
            filesystems: Rc::new(RefCell::new(BidirMap::new())),
            block_devs: Rc::new(RefCell::new(BidirMap::new())),
            cache_devs: Rc::new(RefCell::new(BidirMap::new())),
            next_index: Rc::new(Cell::new(0)),
            pools: Rc::new(RefCell::new(BidirMap::new())),
        }
    }
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

/// Get object path from pool name
fn pool_name_to_object_path(dbus_context: &DbusContext,
                            name: &String)
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

/// Get name for pool from object path
fn object_path_to_pool_name(dbus_context: &DbusContext,
                            path: &String)
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

/// Macro for early return with an Ok dbus message on a dbus internal error
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

/// Macro for early return with an Ok dbus message on an engine error
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

/// Translates an engine ErrorEnum to a dbus ErrorEnum.
fn engine_to_dbus_enum(err: &engine::ErrorEnum) -> (ErrorEnum, String) {
    match *err {
        engine::ErrorEnum::Ok => (ErrorEnum::OK, err.get_error_string()),
        engine::ErrorEnum::Error(_) => (ErrorEnum::ERROR, err.get_error_string()),
        engine::ErrorEnum::AlreadyExists(_) => (ErrorEnum::ALREADY_EXISTS, err.get_error_string()),
        engine::ErrorEnum::Busy(_) => (ErrorEnum::BUSY, err.get_error_string()),
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

fn list_pools(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.engine;

    let result = engine.borrow().list_pools();

    let return_message = m.msg.method_return();
    let return_sig = "s";

    let msg = match result {
        Ok(pool_tree) => {
            let msg_vec =
                pool_tree.keys().map(|key| MessageItem::Str(format!("{}", key))).collect();
            let item_array = MessageItem::Array(msg_vec, return_sig.into());
            let (rc, rs) = ok_message_items();
            return_message.append3(item_array, rc, rs)
        }
        Err(x) => {
            let item_array = MessageItem::Array(vec![], return_sig.into());
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(item_array, rc, rs)
        }
    };
    Ok(vec![msg])
}

fn create_dbus_filesystem<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_snapshot_method = f.method(CREATE_SNAPSHOT, (), create_snapshot)
        .in_arg(("name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let rename_method = f.method(RENAME, (), rename_filesystem)
        .in_arg(("name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_mountpoint_method = f.method(SET_MOUNTPOINT, (), set_filesystem_mountpoint)
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let set_quota_method = f.method(SET_QUOTA, (), set_filesystem_quota)
        .out_arg(("quota", "s"))
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

    let filesystems: Array<(&str, &str, u64), _> = try!(iter.read::<Array<(&str, &str, u64), _>>()
        .map_err(|_| MethodErr::invalid_arg(&0)));

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(oqs)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, &object_path.to_string());
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let ref mut list_rc = ErrorEnum::OK;

    let mut vec = Vec::new();

    for (name, mountpoint, size) in filesystems {
        let result = pool.create_filesystem(name, mountpoint, size);

        match result {
            Ok(_) => {
                let object_path: dbus::Path = create_dbus_filesystem(dbus_context.clone());
                dbus_context.filesystems.borrow_mut().insert(object_path.to_string(),
                                                             ((&pool_name).clone(),
                                                              String::from(name)));
                let (rc, rs) = ok_message_items();
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);

            }
            Err(x) => {
                *list_rc = ErrorEnum::LIST_FAILURE;
                let object_path: dbus::Path = default_object_path();
                let (rc, rs) = engine_to_dbus_err(&x);
                let (rc, rs) = code_to_message_items(rc, rs);
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);
            }
        };
    }

    let (rc, rs) = code_to_message_items(*list_rc, list_rc.get_error_string().into());

    Ok(vec![return_message.append3(MessageItem::Array(vec, return_sig.into()), rc, rs)])

}

fn create_snapshot(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn set_filesystem_quota(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn set_filesystem_mountpoint(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn rename_filesystem(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn list_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "s";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, &object_path.to_string());
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let result = pool.list_filesystems();

    let msg = match result {
        Ok(filesystem_tree) => {
            let msg_vec =
                filesystem_tree.keys().map(|key| MessageItem::Str(format!("{}", key))).collect();
            let item_array = MessageItem::Array(msg_vec, return_sig.into());
            let (rc, rs) = ok_message_items();
            return_message.append3(item_array, rc, rs)
        }
        Err(x) => {
            let item_array = MessageItem::Array(vec![], return_sig.into());
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(item_array, rc, rs)
        }
    };

    Ok(vec![msg])
}

fn list_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn list_cache(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn list_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn remove_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn create_dbus_blockdev<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let object_path = f.object_path(object_name, dbus_context.clone())
        .introspectable()
        .add(f.interface(STRATIS_DEV_BASE_INTERFACE, ())
            .add_p(f.property::<u64, _>("Size", ())));

    let path = object_path.get_name().to_owned();
    dbus_context.action_list.borrow_mut().push(DeferredAction::Add(object_path));
    path
}

fn add_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let devs: Array<(&str), _> = try!(iter.read::<Array<(&str), _>>()
        .map_err(|_| MethodErr::invalid_arg(&0)));

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(oqs)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, &object_path.to_string());
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let ref mut list_rc = ErrorEnum::OK;
    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();
    let mut vec = Vec::new();

    for dev in blockdevs {
        let result = pool.add_blockdev(dev);
        match result {
            Ok(_) => {
                let object_path: dbus::Path = create_dbus_blockdev(dbus_context.clone());
                dbus_context.block_devs.borrow_mut().insert(object_path.to_string(),
                                                            ((&pool_name).clone(),
                                                             String::from(dev.to_str().unwrap())));
                let (rc, rs) = ok_message_items();
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);

            }
            Err(x) => {
                *list_rc = ErrorEnum::LIST_FAILURE;
                let object_path: dbus::Path = default_object_path();
                let (rc, rs) = engine_to_dbus_err(&x);
                let (rc, rs) = code_to_message_items(rc, rs);
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);
            }
        };
    }
    let (rc, rs) = code_to_message_items(*list_rc, list_rc.get_error_string().into());

    Ok(vec![return_message.append3(MessageItem::Array(vec, return_sig.into()), rc, rs)])
}

fn create_dbus_cachedev<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.get_next_id().to_string());

    let object_path = f.object_path(object_name, dbus_context.clone())
        .introspectable()
        .add(f.interface(STRATIS_CACHE_BASE_INTERFACE, ())
            .add_p(f.property::<u64, _>("Size", ())));

    let path = object_path.get_name().to_owned();
    dbus_context.action_list.borrow_mut().push(DeferredAction::Add(object_path));
    path
}

fn add_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    let cache_devs: Array<(&str), _> = try!(iter.read::<Array<(&str), _>>()
        .map_err(|_| MethodErr::invalid_arg(&0)));

    let dbus_context = m.path.get_data();
    let object_path = m.path.get_name();
    let return_message = message.method_return();
    let return_sig = "(oqs)";
    let default_return = MessageItem::Array(vec![], return_sig.into());

    let pool_name = dbus_try!(
        object_path_to_pool_name(dbus_context, &object_path.to_string());
        default_return; return_message);

    let mut b_engine = dbus_context.engine.borrow_mut();
    let ref mut pool = engine_try!(b_engine.get_pool(&pool_name);
                                   default_return;
                                   return_message);

    let ref mut list_rc = ErrorEnum::OK;
    let blockdevs = cache_devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();
    let mut vec = Vec::new();

    for dev in blockdevs {
        let result = pool.add_cachedev(dev);
        match result {
            Ok(_) => {
                let object_path: dbus::Path = create_dbus_cachedev(dbus_context.clone());
                dbus_context.cache_devs.borrow_mut().insert(object_path.to_string(),
                                                            ((&pool_name).clone(),
                                                             String::from(dev.to_str().unwrap())));
                let (rc, rs) = ok_message_items();
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);

            }
            Err(x) => {
                *list_rc = ErrorEnum::LIST_FAILURE;
                let object_path: dbus::Path = default_object_path();
                let (rc, rs) = engine_to_dbus_err(&x);
                let (rc, rs) = code_to_message_items(rc, rs);
                let entry = MessageItem::Struct(vec![MessageItem::ObjectPath(object_path), rc, rs]);
                vec.push(entry);
            }
        };
    }
    let (rc, rs) = code_to_message_items(*list_rc, list_rc.get_error_string().into());

    Ok(vec![return_message.append3(MessageItem::Array(vec, return_sig.into()), rc, rs)])
}
fn remove_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn create_dbus_pool<'a>(mut dbus_context: DbusContext) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_filesystems_method = f.method(CREATE_FILESYSTEMS, (), create_filesystems)
        .in_arg(("filesystems", "a(sst)"))
        .out_arg(("results", "a(oqs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_filesystems_method = f.method(DESTROY_FILESYSTEMS, (), destroy_filesystems)
        .in_arg(("filesystems", "a(sqs)"))
        .out_arg(("results", "a(qs)"))
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
        .in_arg(("cache_devs", "as"))
        .out_arg(("results", "a(oqs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let remove_cache_devs_method = f.method(REMOVE_CACHE_DEVS, (), remove_cache_devs)
        .in_arg(("cache_devs", "as"))
        .out_arg(("results", "a(qs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let list_devs_method = f.method(LIST_DEVS, (), list_devs)
        .out_arg(("devs", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let add_devs_method = f.method(ADD_DEVS, (), add_devs)
        .in_arg(("devs", "as"))
        .out_arg(("results", "a(oqs)"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let remove_devs_method = f.method(REMOVE_DEVS, (), remove_devs)
        .in_arg(("devs", "as"))
        .out_arg(("results", "a(qs)"))
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
            .add_m(remove_devs_method));

    let path = object_path.get_name().to_owned();
    dbus_context.action_list.borrow_mut().push(DeferredAction::Add(object_path));
    path
}

fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();

    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    }
    let name: &str = try!(iter.read::<&str>().map_err(|_| MethodErr::invalid_arg(&0)));

    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    }
    let raid_level: u16 = try!(iter.read::<u16>().map_err(|_| MethodErr::invalid_arg(&1)));

    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    }
    let devs: Array<&str, _> = try!(iter.read::<Array<&str, _>>()
        .map_err(|_| MethodErr::invalid_arg(&2)));

    let blockdevs = devs.map(|x| Path::new(x)).collect::<Vec<&Path>>();

    let dbus_context = m.path.get_data();
    let result = dbus_context.engine.borrow_mut().create_pool(name, &blockdevs, raid_level);

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => {
            let object_path: dbus::Path = create_dbus_pool(dbus_context.clone());
            let (rc, rs) = ok_message_items();
            dbus_context.pools.borrow_mut().insert(object_path.to_string(), String::from(name));
            return_message.append3(MessageItem::ObjectPath(object_path), rc, rs)
        }
        Err(x) => {
            let object_path: dbus::Path = default_object_path();
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(MessageItem::ObjectPath(object_path), rc, rs)
        }
    };
    Ok(vec![msg])
}

fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    }
    let name: &str = try!(iter.read::<&str>().map_err(|_| MethodErr::invalid_arg(&0)));

    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.engine;
    let result = engine.borrow_mut().destroy_pool(&name);

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

fn get_pool_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let message: &Message = m.msg;
    let mut iter = message.iter_init();
    if iter.arg_type() == 0 {
        return Err(MethodErr::no_arg());
    }
    let name: &str = try!(iter.read::<&str>().map_err(|_| MethodErr::invalid_arg(&0)));

    let dbus_context = m.path.get_data();
    let return_message = message.method_return();
    let default_return = MessageItem::ObjectPath(default_object_path());
    let result = pool_name_to_object_path(dbus_context, &name.to_string());
    let object_path = dbus_try!(result; default_return; return_message);

    let path = dbus::Path::new(object_path).unwrap();
    let (rc, rs) = ok_message_items();
    Ok(vec![return_message.append3(MessageItem::ObjectPath(path), rc, rs)])
}

fn get_filesystem_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/filesystem/path", 0, "Ok")])
}

fn get_dev_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/dev/path", 0, "Ok")])
}

fn get_cache_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
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

fn get_dev_types(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return()])
}

fn get_base_tree<'a>(dbus_context: DbusContext) -> StratisResult<Tree<MTFn<TData>, TData>> {

    let f = Factory::new_fn();

    let base_tree = f.tree();

    let create_pool_method = f.method(CREATE_POOL, (), create_pool)
        .in_arg(("pool_name", "s"))
        .in_arg(("raid_type", "q"))
        .in_arg(("dev_list", "as"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroy_pool_method = f.method(DESTROY_POOL, (), destroy_pool)
        .in_arg(("pool_name", "s"))
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

    let get_dev_object_path_method = f.method(GET_DEV_OBJECT_PATH, (), get_dev_object_path)
        .in_arg(("dev_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let get_cache_object_path_method = f.method(GET_CACHE_OBJECT_PATH, (), get_cache_object_path)
        .in_arg(("cache_dev_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let get_error_codes_method = f.method(GET_ERROR_CODES, (), get_error_codes)
        .out_arg(("error_codes", "a(sqs)"));

    let get_raid_levels_method = f.method(GET_RAID_LEVELS, (), get_raid_levels)
        .out_arg(("error_codes", "a(sqs)"));

    let get_dev_types_method = f.method(GET_DEV_TYPES, (), get_dev_types);

    let obj_path = f.object_path(STRATIS_BASE_PATH, dbus_context)
        .introspectable()
        .object_manager()
        .add(f.interface(STRATIS_MANAGER_INTERFACE, ())
            .add_m(list_pools_method)
            .add_m(create_pool_method)
            .add_m(destroy_pool_method)
            .add_m(get_pool_object_path_method)
            .add_m(get_filesystem_object_path_method)
            .add_m(get_dev_object_path_method)
            .add_m(get_cache_object_path_method)
            .add_m(get_error_codes_method)
            .add_m(get_raid_levels_method)
            .add_m(get_dev_types_method));

    let base_tree = base_tree.add(obj_path);

    Ok(base_tree)
}

pub fn run(engine: Rc<RefCell<Engine>>) -> StratisResult<()> {
    let dbus_context = DbusContext::new(&engine);
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
                        tree.remove(path.get_name());
                        c.unregister_object_path(path.get_name());
                    }
                }
            }
        }
    }
    Ok(())
}
