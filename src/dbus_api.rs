// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Display;
use std::path::PathBuf;
use std::rc::Rc;
use std::slice::Iter;
use std::string::String;
use std::sync::Arc;

use dbus;
use dbus::Connection;
use dbus::Message;
use dbus::MessageItem;
use dbus::NameFlag;

use dbus::tree::Factory;
use dbus::tree::MethodErr;
use dbus::tree::MethodFn;
use dbus::tree::MethodResult;
use dbus::tree::Property;
use dbus::tree::Tree;

use dbus_consts::*;

use engine::Engine;
use types::{StratisResult, StratisError};

#[derive(Debug, Clone)]
pub struct DbusContext<'a> {
    name_prop: Arc<Property<MethodFn<'a>>>,
    pub remaining_prop: Arc<Property<MethodFn<'a>>>,
    pub total_prop: Arc<Property<MethodFn<'a>>>,
    pub status_prop: Arc<Property<MethodFn<'a>>>,
    pub running_status_prop: Arc<Property<MethodFn<'a>>>,
    pub block_devices_prop: Arc<Property<MethodFn<'a>>>,
}

impl<'a> DbusContext<'a> {
    pub fn update_one(prop: &Arc<Property<MethodFn<'a>>>, m: MessageItem) -> StratisResult<()> {
        match prop.set_value(m) {
            Ok(_) => Ok(()), // TODO: return signals
            Err(()) => Err(StratisError::Dbus(dbus::Error::new_custom(
                "UpdateError", "Could not update property with value"))),
        }
    }
}

fn list_pools(m: &Message, engine: &Rc<RefCell<Engine>>) -> MethodResult {

    Ok(vec![m.method_return()])
}

fn create_pool(m: &Message, engine: &Rc<RefCell<Engine>>) -> MethodResult {

    let mut items = m.get_items();
    if items.len() < 1 {
        return Err(MethodErr::no_arg());
    }

    let raid_level: u16 = try!(items.pop()
        .ok_or_else(MethodErr::no_arg)
        .and_then(|i| {
            i.inner()
                .map_err(|_| MethodErr::invalid_arg(&i))
        }));

    let devs = match try!(items.pop().ok_or_else(MethodErr::no_arg)) {
        MessageItem::Array(x, _) => x,
        x => return Err(MethodErr::invalid_arg(&x)),
    };

    // Get the name of the pool from the parameters
    let name = try!(items.pop()
        .ok_or_else(MethodErr::no_arg)
        .and_then(|i| {
            i.inner::<&str>()
                .map_err(|_| MethodErr::invalid_arg(&i))
                .map(|i| i.to_owned())
        }));

    let blockdevs = devs.into_iter()
        .map(|x| PathBuf::from(x.inner::<&str>().unwrap()))
        .collect::<Vec<_>>();

    let result = engine.borrow().create_pool(&name, &blockdevs, raid_level);

    Ok(vec![m.method_return().append3("/dbus/newpool/path", 0, "Ok")])
}

fn destroy_pool(m: &Message, engine: &Rc<RefCell<Engine>>) -> MethodResult {

    let mut items = m.get_items();
    if items.len() < 1 {
        return Err(MethodErr::no_arg());
    }

    // Get the name of the pool from the parameters
    let name = try!(items.pop()
        .ok_or_else(MethodErr::no_arg)
        .and_then(|i| {
            i.inner::<&str>()
                .map_err(|_| MethodErr::invalid_arg(&i))
                .map(|i| i.to_owned())
        }));

    let result = engine.borrow().destroy_pool(&name);

    Ok(vec![m.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn get_pool_object_path(m: &Message) -> MethodResult {

    Ok(vec![m.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn get_volume_object_path(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/volume/path", 0, "Ok")])
}

fn get_dev_object_path(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/dev/path", 0, "Ok")])
}

fn get_cache_object_path(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/cache/path", 0, "Ok")])
}

fn get_list_items<T: HasCodes + Display>(m: &Message, items: &Iter<T>) -> MethodResult {

    let msg_vec = items.as_slice()
        .into_iter()
        .map(|item| {
            MessageItem::Struct(vec![MessageItem::Str(format!("{}", item)),
                                     MessageItem::UInt16(item.get_error_int()),
                                     MessageItem::Str(format!("{}", item.get_error_string()))])
        })
        .collect::<Vec<MessageItem>>();

    let item_array = MessageItem::Array(msg_vec, Cow::Borrowed("(sqs)"));
    Ok(vec![m.method_return().append1(item_array)])
}

fn get_error_codes(m: &Message) -> MethodResult {
    get_list_items(m, &StratisErrorEnum::iterator())
}


fn get_raid_levels(m: &Message) -> MethodResult {
    get_list_items(m, &StratisRaidType::iterator())
}

fn get_dev_types(m: &Message) -> MethodResult {
    let mut items = m.get_items();
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

    Ok(vec![m.method_return()])
}

pub fn get_base_tree<'a>(c: &'a Connection,
                         engine: Rc<RefCell<Engine>>)
                         -> StratisResult<Tree<MethodFn<'a>>> {
    c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32).unwrap();

    let f = Factory::new_fn();

    let base_tree = f.tree();

    let engine_clone = engine.clone();

    let createpool_method = f.method(CREATE_POOL, move |m, _, _| create_pool(m, &engine_clone))
        .in_arg(("pool_name", "s"))
        .in_arg(("dev_list", "as"))
        .in_arg(("raid_type", "q"))
        .out_arg(("object_path", "s"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let engine_clone = engine.clone();

    let destroypool_method = f.method(DESTROY_POOL, move |m, _, _| destroy_pool(m, &engine_clone))
        .in_arg(("pool_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let engine_clone = engine.clone();

    let listpools_method = f.method(LIST_POOLS, move |m, _, _| list_pools(m, &engine_clone))
        .out_arg(("pool_names", "as"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getpoolobjectpath_method =
        f.method(GET_POOL_OBJECT_PATH, move |m, _, _| get_pool_object_path(m))
            .in_arg(("pool_name", "s"))
            .out_arg(("object_path", "o"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"));

    let getvolumeobjectpath_method = f.method(GET_VOLUME_OBJECT_PATH,
                move |m, _, _| get_volume_object_path(m))
        .in_arg(("pool_name", "s"))
        .in_arg(("volume_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let getdevobjectpath_method =
        f.method(GET_DEV_OBJECT_PATH, move |m, _, _| get_dev_object_path(m))
            .in_arg(("dev_name", "s"))
            .out_arg(("object_path", "o"))
            .out_arg(("return_code", "q"))
            .out_arg(("return_string", "s"));

    let getcacheobjectpath_method = f.method(GET_CACHE_OBJECT_PATH,
                move |m, _, _| get_cache_object_path(m))
        .in_arg(("cache_dev_name", "s"))
        .out_arg(("object_path", "o"))
        .out_arg(("return_code", "q"))
        .out_arg(("return_string", "s"));

    let geterrorcodes_method = f.method(GET_ERROR_CODES, move |m, _, _| get_error_codes(m))
        .out_arg(("error_codes", "a(sqs)"));

    let getraidlevels_method = f.method(GET_RAID_LEVELS, move |m, _, _| get_raid_levels(m))
        .out_arg(("error_codes", "a(sqs)"));

    let getdevtypes_method = f.method(GET_DEV_TYPES, move |m, _, _| get_dev_types(m));


    let obj_path = f.object_path(STRATIS_BASE_PATH)
        .introspectable()
        .add(f.interface(STRATIS_MANAGER_INTERFACE)
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
    try!(base_tree.set_registered(c, true));

    Ok(base_tree)
}
