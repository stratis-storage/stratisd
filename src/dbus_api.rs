// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Display;
use std::path::Path;
use std::rc::Rc;
use std::io::ErrorKind;
use std::collections::BTreeMap;
use std::fmt;

use dbus::Connection;
use dbus::BusType;
use dbus::MessageItem;
use dbus::NameFlag;
use dbus::tree::Factory;
use dbus::tree::DataType;
use dbus::tree::MethodErr;
use dbus::tree::MTFn;
use dbus::tree::MethodResult;
use dbus::tree::MethodInfo;
use dbus::tree::Tree;

use dbus_consts::*;

use engine::Engine;
use types::{StratisResult, StratisError};

#[derive(Clone)]
pub struct DbusContext {
    pub next_index: i32,
    pub pools: BTreeMap<String, String>,
    pub engine: Rc<RefCell<Engine>>,
}

// engine doesn't impl Debug, so we can't derive
impl fmt::Debug for DbusContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{DBusContext {:?}, {:?}}}", self.next_index, self.pools)
    }
}

impl DbusContext {
    pub fn new(engine: &Rc<RefCell<Engine>>) -> DbusContext {
        DbusContext {
            next_index: 0,
            pools: BTreeMap::new(),
            engine: engine.clone(),
        }
    }
    pub fn get_next_id(&mut self) {
        self.next_index += 1;
    }
}

#[derive(Copy, Clone, Default, Debug)]
struct TData;
impl DataType for TData {
    type ObjectPath = Rc<RefCell<DbusContext>>;
    type Property = ();
    type Interface = ();
    type Method = ();
    type Signal = ();
}

fn internal_to_dbus_err(err: &StratisError) -> StratisErrorEnum {
    match *err {
        StratisError::Stratis(_) => StratisErrorEnum::STRATIS_ERROR,
        StratisError::Io(ref err) => {
            match err.kind() {
                ErrorKind::NotFound => StratisErrorEnum::STRATIS_NOTFOUND,
                ErrorKind::AlreadyExists => StratisErrorEnum::STRATIS_ALREADY_EXISTS,
                _ => StratisErrorEnum::STRATIS_ERROR,
            }
        }
        _ => StratisErrorEnum::STRATIS_ERROR,
    }
}

fn list_pools(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.borrow().engine;
    let result = engine.borrow().list_pools();

    let mut msg_vec = Vec::new();

    // TODO: deal with failure
    let pool_tree = result.unwrap();

    for (name, pool) in pool_tree {
        let entry = vec![MessageItem::Str(format!("{}", name)),
                         MessageItem::UInt16(0),
                         MessageItem::Str(String::from("Ok"))];
        msg_vec.push(MessageItem::Struct(entry));
    }

    let item_array = MessageItem::Array(msg_vec, Cow::Borrowed("(sqs)"));

    Ok(vec![m.msg.method_return().append1(item_array)])
}

fn create_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let dbus_context = m.path.get_data();
    let ref mut engine = dbus_context.borrow_mut().engine;

    let message: (Option<MessageItem>, Option<MessageItem>, Option<MessageItem>) = m.msg.get3();

    let item0: MessageItem = try!(message.0.ok_or_else(MethodErr::no_arg));
    let name: &String = try!(item0.inner().map_err(|_| MethodErr::invalid_arg(&item0)));

    let item1: MessageItem = try!(message.1.ok_or_else(MethodErr::no_arg));
    let devs: &Vec<MessageItem> = try!(item1.inner().map_err(|_| MethodErr::invalid_arg(&item1)));
    let mut devstrings = devs.iter().map(|x| x.inner::<&String>());
    if devstrings.any(|x| x.is_err()) {
        return Err(MethodErr::invalid_arg(&item1));
    }
    let blockdevs = devstrings.map(|x| Path::new(x.unwrap())).collect::<Vec<&Path>>();

    let item2: MessageItem = try!(message.2.ok_or_else(MethodErr::no_arg));
    let raid_level: u16 = try!(item2.inner().map_err(|_| MethodErr::invalid_arg(&item2)));

    let result = engine.borrow_mut().create_pool(name, &blockdevs, raid_level);
    match result {
        Ok(_) => Ok(vec![m.msg.method_return().append3("/dbus/newpool/path", 0, "Ok")]),
        Err(x) => {
            let dbus_err = internal_to_dbus_err(&x);
            Ok(vec![m.msg
                        .method_return()
                        .append3("", dbus_err.get_error_int(), dbus_err.get_error_string())])
        }
    }
}

fn destroy_pool(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    let dbus_context = m.path.get_data();
    let ref engine = dbus_context.borrow().engine;

    let item: MessageItem = try!(m.msg.get1().ok_or_else(MethodErr::no_arg));

    let name: &String = try!(item.inner().map_err(|_| MethodErr::invalid_arg(&item)));

    let result = engine.borrow_mut().destroy_pool(&name);

    let message = m.msg.method_return();

    let msg = match result {
        Ok(_) => {
            message.append2(MessageItem::UInt16(0),
                            MessageItem::Str(format!("{}", "Ok")))
        }
        Err(err) => {
            message.append2(MessageItem::UInt16(0),
                            MessageItem::Str(format!("{}", "Ok")))
        }
    };

    Ok(vec![msg])
}

fn get_pool_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {

    Ok(vec![m.msg.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn get_volume_object_path(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    Ok(vec![m.msg.method_return().append3("/dbus/volume/path", 0, "Ok")])
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
    get_list_items(m, StratisErrorEnum::iter_variants())
}


fn get_raid_levels(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    get_list_items(m, StratisRaidType::iter_variants())
}

fn get_dev_types(m: &MethodInfo<MTFn<TData>, TData>) -> MethodResult {
    let mut items = m.msg.get_items();
    if items.len() < 1 {
        return Err(MethodErr::no_arg());
    }

    let _name = try!(items.pop()
        .ok_or_else(MethodErr::no_arg)
        .and_then(|i| {
            i.inner::<&str>()
                .map_err(|_| MethodErr::invalid_arg(&i))
                .map(|i| i.to_owned())
        }));

    println!("method called");

    Ok(vec![m.msg.method_return()])
}

fn get_base_tree<'a>(dbus_context: Rc<RefCell<DbusContext>>)
                     -> StratisResult<Tree<MTFn<TData>, TData>> {

    let f = Factory::new_fn();

    let base_tree = f.tree();

    let createpool_method = f.method(CREATE_POOL, (), create_pool)
        .in_arg(("pool_name", "s"))
        .in_arg(("dev_list", "as"))
        .in_arg(("raid_type", "q"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let destroypool_method = f.method(DESTROY_POOL, (), destroy_pool)
        .in_arg(("pool_name", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let listpools_method = f.method(LIST_POOLS, (), list_pools)
        .out_arg(("pool_names", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getpoolobjectpath_method = f.method(GET_POOL_OBJECT_PATH, (), get_pool_object_path)
        .in_arg(("pool_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getvolumeobjectpath_method = f.method(GET_VOLUME_OBJECT_PATH, (), get_volume_object_path)
        .in_arg(("pool_name", "s"))
        .in_arg(("volume_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getdevobjectpath_method = f.method(GET_DEV_OBJECT_PATH, (), get_dev_object_path)
        .in_arg(("dev_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getcacheobjectpath_method = f.method(GET_CACHE_OBJECT_PATH, (), get_cache_object_path)
        .in_arg(("cache_dev_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let geterrorcodes_method = f.method(GET_ERROR_CODES, (), get_error_codes)
        .out_arg(("error_codes", "a(sqs)"));

    let getraidlevels_method = f.method(GET_RAID_LEVELS, (), get_raid_levels)
        .out_arg(("error_codes", "a(sqs)"));

    let getdevtypes_method = f.method(GET_DEV_TYPES, (), get_dev_types);

    let obj_path = f.object_path(STRATIS_BASE_PATH, dbus_context)
        .introspectable()
        .add(f.interface(STRATIS_MANAGER_INTERFACE, ())
            .add_m(listpools_method)
            .add_m(createpool_method)
            .add_m(destroypool_method)
            .add_m(getpoolobjectpath_method)
            .add_m(getvolumeobjectpath_method)
            .add_m(getdevobjectpath_method)
            .add_m(getcacheobjectpath_method)
            .add_m(geterrorcodes_method)
            .add_m(getraidlevels_method)
            .add_m(getdevtypes_method));


    let base_tree = base_tree.add(obj_path);

    Ok(base_tree)
}

pub fn run(engine: Rc<RefCell<Engine>>) -> StratisResult<()> {
    let dbus_context = Rc::new(RefCell::new(DbusContext::new(&engine)));
    let tree = get_base_tree(dbus_context.clone()).unwrap();

    // Setup DBus connection
    let c = try!(Connection::get_private(BusType::Session));
    c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32).unwrap();
    try!(tree.set_registered(&c, true));

    // ...and serve incoming requests.
    for _ in tree.run(&c, c.iter(1000)) {
    }

    Ok(())
}
