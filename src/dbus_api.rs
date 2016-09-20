// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::rc::Rc;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::error::Error;

use dbus::{Connection, NameFlag};
use dbus::tree::{Factory, Tree, Property, MethodFn, MethodErr, EmitsChangedSignal, Interface};
use dbus::MessageItem;
use dbus;
use dbus::Message;
use dbus::tree::MethodResult;

use dbus_consts::*;
use blockdev::{BlockMember, BlockDevs};


use stratis::Stratis;

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

fn createpool(m: &Message) -> MethodResult {


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

    m.method_return().append2("STRATIS_OK", StratisErrorEnum::STRATIS_OK as i32);
	m.method_return().append2("STRATIS_ERROR", StratisErrorEnum::STRATIS_ERROR as i32);
	m.method_return().append2("STRATIS_NULL", StratisErrorEnum::STRATIS_NULL as i32);
	m.method_return().append2("STRATIS_MALLOC", StratisErrorEnum::STRATIS_MALLOC as i32);
	m.method_return().append2("STRATIS_NOTFOUND", StratisErrorEnum::STRATIS_NOTFOUND as i32);
	m.method_return().append2("STRATIS_POOL_NOTFOUND", StratisErrorEnum::STRATIS_POOL_NOTFOUND as i32);
	m.method_return().append2("STRATIS_VOLUME_NOTFOUND",StratisErrorEnum::STRATIS_VOLUME_NOTFOUND as i32 );
	m.method_return().append2("STRATIS_DEV_NOTFOUND", StratisErrorEnum::STRATIS_DEV_NOTFOUND as i32);
	m.method_return().append2("STRATIS_CACHE_NOTFOUND", StratisErrorEnum::STRATIS_CACHE_NOTFOUND as i32);
	m.method_return().append2("STRATIS_BAD_PARAM", StratisErrorEnum::STRATIS_BAD_PARAM as i32);
	m.method_return().append2("STRATIS_ALREADY_EXISTS", StratisErrorEnum::STRATIS_ALREADY_EXISTS as i32);
	m.method_return().append2("STRATIS_NULL_NAME", StratisErrorEnum::STRATIS_NULL_NAME as i32);
	m.method_return().append2("STRATIS_NO_POOLS", StratisErrorEnum::STRATIS_NO_POOLS as i32);
	m.method_return().append2("STRATIS_LIST_FAILURE", StratisErrorEnum::STRATIS_LIST_FAILURE as i32);
	m.method_return().append2("STRATIS_ERROR_MAX", StratisErrorEnum::STRATIS_ERROR_MAX as i32);
	
    Ok(vec![m.method_return()])
}

fn getraidlevels(m: &Message) -> MethodResult {
    let mut items = m.get_items();
    if items.len() < 1 {
        return Err(MethodErr::no_arg());
    }

    let name = try!(items.pop()
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

    let name = try!(items.pop()
        .ok_or_else(MethodErr::no_arg)
        .and_then(|i| {
            i.inner::<&str>()
                .map_err(|_| MethodErr::invalid_arg(&i))
                .map(|i| i.to_owned())
        }));

    println!("method called");

    Ok(vec![m.method_return()])
}
pub fn get_base_tree<'a>(c: &'a Connection) -> StratisResult<Tree<MethodFn<'a>>> {
    c.register_name(STRATIS_BASE_SERVICE, NameFlag::ReplaceExisting as u32).unwrap();

    let f = Factory::new_fn();

    let base_tree = f.tree();

    let listpools_method = f.method(LIST_POOLS, move |m, _, _| { listpools(m) })
         .out_arg(("pool_names", "as"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let createpool_method = f.method(CREATE_POOL, move |m, _, _| { createpool(m) })
         .in_arg(("pool_name", "s"))
         .in_arg(("dev_list", "as"))
         .in_arg(("raid_type", "i"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let destroypool_method = f.method(DESTROY_POOL, move |m, _, _| { destroypool(m) })
         .in_arg(("pool_name", "s"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let getpoolobjectpath_method = f.method(GET_POOL_OBJECT_PATH, move |m, _, _| { getpoolobjectpath(m) })
          .in_arg(("pool_name", "s"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let getvolumeobjectpath_method = f.method(GET_VOLUME_OBJECT_PATH, move |m, _, _| { getvolumeobjectpath(m) })
         .in_arg(("pool_name", "s"))
         .in_arg(("volume_name", "s"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let getdevobjectpath_method = f.method(GET_DEV_OBJECT_PATH, move |m, _, _| { getdevobjectpath(m) })
         .in_arg(("dev_name", "s"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let getcacheobjectpath_method = f.method(GET_CACHE_OBJECT_PATH, move |m, _, _| { getcacheobjectpath(m) })
         .in_arg(("cache_dev_name", "s"))
         .out_arg(("object_path", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));

    let geterrorcodes_method = f.method(GET_ERROR_CODES, move |m, _, _| { geterrorcodes(m) });

    let getraidlevels_method = f.method(GET_RAID_LEVELS, move |m, _, _| { getraidlevels(m) });

    let getdevtypes_method = f.method(GET_DEV_TYPES, move |m, _, _| { getdevtypes(m) });



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
