// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::sync::Arc;
use std::rc::Rc;
use std::cell::RefCell;
use std::borrow::Cow;
use std::string::String;
use dbus::{Connection, NameFlag};
use dbus::tree::{Factory, Tree, Property, MethodFn, MethodErr};
use dbus::MessageItem;
use dbus;
use dbus::TypeSig;
use dbus::Message;
use dbus::tree::MethodResult;

use dbus_consts::*;
use engine::Engine;

// use stratis::Stratis;

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

fn listpools(m: &Message) -> MethodResult {

    m.method_return().append2("pool1", StratisErrorEnum::STRATIS_OK as i32);
    m.method_return().append2("pool2", StratisErrorEnum::STRATIS_OK as i32);
    m.method_return().append2("pool3", StratisErrorEnum::STRATIS_OK as i32);
    m.method_return().append2("pool4", StratisErrorEnum::STRATIS_OK as i32);
    m.method_return().append2("pool5", StratisErrorEnum::STRATIS_OK as i32);
    Ok(vec![m.method_return()])
}

fn createpool(m: &Message, engine: Rc<RefCell<Engine>>) -> MethodResult {

    // create a pool but we don't know what its blockdevs are yet
    let result = engine.borrow().create_pool("abcde", &[]);

    Ok(vec![m.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn destroypool(m: &Message) -> MethodResult {

    Ok(vec![m.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn getpoolobjectpath(m: &Message) -> MethodResult {

    Ok(vec![m.method_return().append3("/dbus/pool/path", 0, "Ok")])
}

fn getvolumeobjectpath(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/volume/path", 0, "Ok")])
}

fn getdevobjectpath(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/dev/path", 0, "Ok")])
}

fn getcacheobjectpath(m: &Message) -> MethodResult {
    Ok(vec![m.method_return().append3("/dbus/cache/path", 0, "Ok")])
}


fn geterrorcodes(m: &Message) -> MethodResult {

    let mut msg_vec = Vec::new();

    for error in StratisErrorEnum::iterator() {
        //      m.method_return().append3(format!("{}", error),
        // StratisErrorEnum::get_error_int(error),
        // StratisErrorEnum::get_error_string(error));


        // let v = MessageItem::Struct(vec![MessageItem::Int32(1),
        // MessageItem::Str(format!("Hello")),
        // MessageItem::Int32(3)]);

        let v = MessageItem::Struct(vec![MessageItem::Str(format!("{}", error)),
                                     MessageItem::Int32(3),
                                     MessageItem::Str( String::from(StratisErrorEnum::get_error_string(error)))]);


        msg_vec.push(v);

    }
    // let mut rv = m.method_return();

    // rv.append(MessageItem::Array(msg_vec, Cow::Borrowed("(iii)")));
    let item = MessageItem::Array(msg_vec, Cow::Borrowed("(sis)"));

    // Ok(vec![rv])

    Ok(vec![m.method_return().append1(item)])

}


fn getraidlevels(m: &Message) -> MethodResult {
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

fn getdevtypes(m: &Message) -> MethodResult {
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

    let listpools_method = f.method(LIST_POOLS, move |m, _, _| listpools(m))
        .out_arg(("pool_names", "as"))
        .out_arg(("return_code", "i"))
        .out_arg(("return_string", "s"));

    let createpool_method = f.method(CREATE_POOL, move |m, _, _| createpool(m, engine.clone()))
        .in_arg(("pool_name", "s"))
        .in_arg(("dev_list", "as"))
        .in_arg(("raid_type", "i"))
        .out_arg(("object_path", "s"))
        .out_arg(("return_code", "i"))
        .out_arg(("return_string", "s"));

    let destroypool_method = f.method(DESTROY_POOL, move |m, _, _| destroypool(m))
        .in_arg(("pool_name", "s"))
        .out_arg(("object_path", "s"))
        .out_arg(("return_code", "i"))
        .out_arg(("return_string", "s"));

    let getpoolobjectpath_method =
        f.method(GET_POOL_OBJECT_PATH, move |m, _, _| getpoolobjectpath(m))
            .in_arg(("pool_name", "s"))
            .out_arg(("object_path", "s"))
            .out_arg(("return_code", "i"))
            .out_arg(("return_string", "s"));

    let getvolumeobjectpath_method = f.method(GET_VOLUME_OBJECT_PATH,
                move |m, _, _| getvolumeobjectpath(m))
        .in_arg(("pool_name", "s"))
        .in_arg(("volume_name", "s"))
        .out_arg(("object_path", "s"))
        .out_arg(("return_code", "i"))
        .out_arg(("return_string", "s"));

    let getdevobjectpath_method = f.method(GET_DEV_OBJECT_PATH, move |m, _, _| getdevobjectpath(m))
        .in_arg(("dev_name", "s"))
        .out_arg(("object_path", "s"))
        .out_arg(("return_code", "i"))
        .out_arg(("return_string", "s"));

    let getcacheobjectpath_method =
        f.method(GET_CACHE_OBJECT_PATH, move |m, _, _| getcacheobjectpath(m))
            .in_arg(("cache_dev_name", "s"))
            .out_arg(("object_path", "s"))
            .out_arg(("return_code", "i"))
            .out_arg(("return_string", "s"));

    let geterrorcodes_method = f.method(GET_ERROR_CODES, move |m, _, _| geterrorcodes(m))
        .out_arg(("error_codes", "a(sis)"));

    let getraidlevels_method = f.method(GET_RAID_LEVELS, move |m, _, _| getraidlevels(m));

    let getdevtypes_method = f.method(GET_DEV_TYPES, move |m, _, _| getdevtypes(m));



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
