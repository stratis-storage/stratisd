// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Display;
use std::path::Path;
use std::rc::Rc;
use std::collections::BTreeMap;

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

#[derive(Debug)]
pub struct DbusContext {
    pub next_index: u64,
    pub pools: BTreeMap<String, String>,
    pub engine: Rc<RefCell<Engine>>,
    pub action_list: Vec<DeferredAction>,
}


impl DbusContext {
    pub fn new(engine: &Rc<RefCell<Engine>>) -> DbusContext {
        DbusContext {
            next_index: 0,
            pools: BTreeMap::new(),
            engine: engine.clone(),
            action_list: Vec::new(),
        }
    }
    pub fn get_next_id(&mut self) -> u64 {
        self.next_index += 1;
        self.next_index
    }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct TData;
impl DataType for TData {
    type ObjectPath = Rc<RefCell<DbusContext>>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}

fn engine_to_dbus_enum(err: &engine::ErrorEnum) -> (ErrorEnum, String) {
    match *err {
        engine::ErrorEnum::Ok => (ErrorEnum::OK, err.get_error_string()),
        engine::ErrorEnum::Error(_) => (ErrorEnum::ERROR, err.get_error_string()),
        engine::ErrorEnum::AlreadyExists(_) => {
            (ErrorEnum::ALREADY_EXISTS, err.get_error_string())
        }
        engine::ErrorEnum::Busy(_) => (ErrorEnum::BUSY, err.get_error_string()),
    }
}

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

fn code_to_message_items(code: ErrorEnum, mes: String) -> (MessageItem, MessageItem) {
    (MessageItem::UInt16(code.get_error_int()), MessageItem::Str(mes))
}

fn ok_message_items() -> (MessageItem, MessageItem) {
    let code = ErrorEnum::OK;
    code_to_message_items(code, code.get_error_string().into())
}

fn default_object_path<'a>() -> dbus::Path<'a> {
    dbus::Path::new(DEFAULT_OBJECT_PATH).unwrap()
}

fn list_pools(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.borrow().engine;
    let result = engine.borrow().list_pools();

    let return_message = m.msg.method_return();

    let msg = match result {
        Ok(pool_tree) => {
            let msg_vec =
                pool_tree.keys().map(|key| MessageItem::Str(format!("{}", key))).collect();
            let item_array = MessageItem::Array(msg_vec, "s".into());
            let (rc, rs) = ok_message_items();
            return_message.append3(item_array, rc, rs)
        }
        Err(x) => {
            let item_array = MessageItem::Array(vec![], "s".into());
            let (rc, rs) = engine_to_dbus_err(&x);
            let (rc, rs) = code_to_message_items(rc, rs);
            return_message.append3(item_array, rc, rs)
        }
    };
    Ok(vec![msg])
}


fn create_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn destroy_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn list_filesystems(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
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

fn add_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn remove_cache_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn add_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn remove_devs(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn create_dbus_pool<'a>(dbus_context: Rc<RefCell<DbusContext>>) -> dbus::Path<'a> {

    let f = Factory::new_fn();

    let create_filesystems_method = f.method(CREATE_FILESYSTEMS, (), create_filesystems);

    let destroy_filesystems_method = f.method(DESTROY_FILESYSTEMS, (), destroy_filesystems);

    let list_filesystems_method = f.method(LIST_FILESYSTEMS, (), list_filesystems);

    let list_devs_method = f.method(LIST_DEVS, (), list_devs);

    let list_cache_devs_method = f.method(LIST_CACHE_DEVS, (), list_cache_devs);

    let add_cache_devs_method = f.method(ADD_CACHE_DEVS, (), add_cache_devs);

    let remove_cache_devs_method = f.method(REMOVE_CACHE_DEVS, (), remove_cache_devs);

    let add_devs_method = f.method(ADD_DEVS, (), add_devs);

    let remove_devs_method = f.method(REMOVE_DEVS, (), remove_devs);

    let object_name = format!("{}/{}",
                              STRATIS_BASE_PATH,
                              dbus_context.borrow_mut().get_next_id().to_string());

    let object_path = f.object_path(object_name, dbus_context.clone())
        .introspectable()
        .add(f.interface(STRATIS_MANAGER_INTERFACE, ())
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
    dbus_context.borrow_mut().action_list.push(DeferredAction::Add(object_path));
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
    let result = {
        let ref mut engine = dbus_context.borrow_mut().engine;
        let result = engine.borrow_mut().create_pool(name, &blockdevs, raid_level);
        result
    };

    let return_message = message.method_return();

    let msg = match result {
        Ok(_) => {
            let dbus_context_clone = dbus_context.clone();
            let object_path: dbus::Path = create_dbus_pool(dbus_context_clone);
            let (rc, rs) = ok_message_items();
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
    let ref engine = dbus_context.borrow().engine;
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

    Ok(vec![m.msg.method_return().append3("/dbus/pool/path", 0, "Ok")])
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

fn get_base_tree<'a>(dbus_context: Rc<RefCell<DbusContext>>)
                     -> StratisResult<Tree<MTFn<TData>, TData>> {

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
    let dbus_context = Rc::new(RefCell::new(DbusContext::new(&engine)));
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
            let mut cxt = dbus_context.borrow_mut();
            for action in cxt.action_list.drain(..) {
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
