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

use blockdev::{BlockMember, BlockDevs};

use consts::*;

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

fn createpool(m: &Message) -> MethodResult {
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

fn destroypool(m: &Message) -> MethodResult {
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

fn getpoolobjectpath(m: &Message) -> MethodResult {
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

fn getvolumeobjectpath(m: &Message) -> MethodResult {
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

fn getdevobjectpath(m: &Message) -> MethodResult {
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

fn getcacheobjectpath(m: &Message) -> MethodResult {
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

fn geterrorcodes(m: &Message) -> MethodResult {
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
         .in_arg(("poolname", "s"))
         .in_arg(("devlist", "as"))
         .in_arg(("raidtype", "i"))
         .out_arg(("object_name", "s"))
         .out_arg(("return_code", "i"))
         .out_arg(("return_string", "s"));
        
    let createpool_method = f.method(CREATE_POOL, move |m, _, _| { createpool(m) })
         .in_arg(("name", "s"));
          
    let destroypool_method = f.method(DESTROY_POOL, move |m, _, _| { destroypool(m) })
         .in_arg(("name", "s"));
         
    let getpoolobjectpath_method = f.method(GET_POOL_OBJECT_PATH, move |m, _, _| { getpoolobjectpath(m) })
         .in_arg(("name", "s"));
         
    let getvolumeobjectpath_method = f.method(GET_VOLUME_OBJECT_PATH, move |m, _, _| { getvolumeobjectpath(m) })
         .in_arg(("name", "s"));
    
    let getdevobjectpath_method = f.method(GET_DEV_OBJECT_PATH, move |m, _, _| { getdevobjectpath(m) })
         .in_arg(("name", "s"));
    
    let getcacheobjectpath_method = f.method(GET_CACHE_OBJECT_PATH, move |m, _, _| { getcacheobjectpath(m) })
         .in_arg(("name", "s"));
    
    let geterrorcodes_method = f.method(GET_ERROR_CODES, move |m, _, _| { geterrorcodes(m) })
         .in_arg(("name", "s"));
    
    let getraidlevels_method = f.method(GET_RAID_LEVELS, move |m, _, _| { getraidlevels(m) })
         .in_arg(("name", "s"));
    
    let getdevtypes_method = f.method(GET_DEV_TYPES, move |m, _, _| { getdevtypes(m) })
         .in_arg(("name", "s"));



    let obj_path = f.object_path(STRATIS_BASE_PATH)
        .introspectable()
        .add(f.interface(STRATIS_BASE_SERVICE)
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
