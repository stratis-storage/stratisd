// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(dead_code)] // only temporary, until more stuff is filled in

extern crate devicemapper;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate byteorder;
extern crate uuid;
extern crate time;
extern crate bytesize;
extern crate dbus;
extern crate term;

#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate newtype_derive;

pub static mut debug: bool = false;

macro_rules! dbgp {
    ($($arg:tt)*) => (
        unsafe {
            if ::debug {
                println!($($arg)*)
            }
        })
}

mod types;
mod consts;
mod dbus_consts;
mod stratis;
mod dmdevice;
mod util;
mod dbus_api;
mod blockdev;
mod pool;
mod filesystem;
mod engine;
mod sim_engine;

use std::io::Write;
use std::error::Error;
use std::process::exit;
use std::path::{Path, PathBuf};
use std::borrow::Cow;
use std::rc::Rc;
use std::cell::RefCell;

use bytesize::ByteSize;
use dbus::{Connection, BusType, Message, MessageItem, FromMessageItem, Props};
use dbus::{ConnectionItem, MessageType};
use time::{Timespec, Duration};

use types::{StratisResult, StratisError, InternalError};
use dbus_consts::DBUS_TIMEOUT;
use consts::SECTOR_SIZE;

use clap::ArgMatches;
use engine::Engine;
use sim_engine::SimEngine;

// We are given BlockDevs to start.
// We allocate LinearDevs from each for the meta and data devices.
// We use all these to make RaidDevs.
// We create two RaidLinearDevs from these for meta and data devices.
// We use these to make a ThinPoolDev.
// From that, we allocate a ThinDev.

trait StratisDbusConnection {
    fn stratis_connect() -> StratisResult<Connection>;
    fn stratis_paths(&self) -> StratisResult<Vec<String>>;
    fn stratis_path(&self, name: &str) -> StratisResult<String>;
}

impl StratisDbusConnection for Connection {
    fn stratis_connect() -> StratisResult<Connection> {
        let c = try!(Connection::get_private(BusType::Session));
        Ok(c)
    }
    fn stratis_paths(&self) -> StratisResult<Vec<String>> {
        let m = Message::new_method_call("org.freedesktop.Stratis1",
                                         "/org/freedesktop/Stratisdevs",
                                         "org.freedesktop.DBus.ObjectManager",
                                         "GetManagedObjects")
            .unwrap();
        let r = try!(self.send_with_reply_and_block(m, DBUS_TIMEOUT));
        let reply = r.get_items();

        let mut pools = Vec::new();
        let array: &Vec<MessageItem> = FromMessageItem::from(&reply[0]).unwrap();
        for item in array {
            let (k, _) = FromMessageItem::from(item).unwrap();
            let kstr: &str = FromMessageItem::from(k).unwrap();
            if kstr != "/org/freedesktop/Stratisdevs" {
                pools.push(kstr.to_owned());
            }
        }
        Ok(pools)
    }

    fn stratis_path(&self, name: &str) -> StratisResult<String> {
        let pools = try!(self.stratis_paths());

        for fpath in &pools {
            let p = Props::new(self,
                               "org.freedesktop.Stratis1",
                               fpath,
                               "org.freedesktop.StratisDevice1",
                               DBUS_TIMEOUT);
            let item = p.get("Name").unwrap();
            let stratis_name: &str = FromMessageItem::from(&item).unwrap();
            if name == stratis_name {
                return Ok(fpath.to_owned());
            }
        }

        Err(StratisError::Stratis(InternalError(format!("Stratisdev \"{}\" not found", name)
            .into())))
    }
}

fn list(_args: &ArgMatches) -> StratisResult<()> {
    let c = try!(Connection::stratis_connect());
    let pools = try!(c.stratis_paths());

    for fpath in &pools {
        let p = Props::new(&c,
                           "org.freedesktop.Stratis1",
                           fpath,
                           "org.freedesktop.StratisDevice1",
                           DBUS_TIMEOUT);
        let item = try!(p.get("Name"));
        let name: &str = FromMessageItem::from(&item).unwrap();
        println!("{}", name);
    }

    Ok(())
}

fn status(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdevname").unwrap();
    let c = try!(Connection::stratis_connect());
    let fpath = try!(c.stratis_path(name));
    let p = Props::new(&c,
                       "org.freedesktop.Stratis1",
                       fpath,
                       "org.freedesktop.StratisDevice1",
                       DBUS_TIMEOUT);
    let status_msg = try!(p.get("Status"));
    let status: u32 = FromMessageItem::from(&status_msg).unwrap();
    let r_status_msg = try!(p.get("RunningStatus"));
    let r_status: u32 = FromMessageItem::from(&r_status_msg).unwrap();

    let stat_str: Cow<str> = {
        if status != 0 {
            let mut stats: Vec<Cow<_>> = Vec::new();
            if 0xff & status != 0 {
                stats.push(format!("stopped, need {} blockdevs", status & 0xff).into())
            }
            if 0x100 & status != 0 {
                stats.push("RAID failure".into())
            }
            if 0x200 & status != 0 {
                stats.push("Thin pool failure: metadata".into())
            }
            if 0x400 & status != 0 {
                stats.push("Thin pool failure: data".into())
            }
            if 0x800 & status != 0 {
                stats.push("Thin device failure".into())
            }
            if 0x1000 & status != 0 {
                stats.push("Filesystem failure".into())
            }
            if 0x2000 & status != 0 {
                stats.push("Initializing".into())
            }
            if 0xffffc000 & status != 0 {
                stats.push(format!("Unenumerated failure: {:x}", status).into())
            }
            stats.join(", ").into()
        } else if r_status != 0 {
            let mut stats: Vec<Cow<_>> = Vec::new();
            if 0xff & r_status != 0 {
                stats.push(format!("missing {} blockdevs", r_status & 0xff).into())
            }
            if 0x100 & r_status != 0 {
                stats.push("Non-redundant".into())
            }
            if 0x200 & r_status != 0 {
                stats.push("Cannot reshape".into())
            } else {
                stats.push("Can reshape".into())
            }
            if 0x400 & r_status != 0 {
                stats.push("Reshaping".into())
            }
            if 0x800 & r_status != 0 {
                stats.push("Throttled".into())
            }
            if 0xfffff000 & r_status != 0 {
                stats.push(format!("Unenumerated issue: {:x}", r_status).into())
            }
            stats.join(", ").into()

        } else {
            "Running".into()
        }
    };

    let space_msg = try!(p.get("RemainingSectors"));
    let space: u64 = FromMessageItem::from(&space_msg).unwrap();
    let space = space * SECTOR_SIZE;

    let total_msg = try!(p.get("TotalSectors"));
    let total: u64 = FromMessageItem::from(&total_msg).unwrap();
    let total = total * SECTOR_SIZE;

    let percent = ((total - space) * 100) / total;

    println!("Status: {}, {}% used ({} of {} free)",
             stat_str,
             percent,
             ByteSize::b(space as usize).to_string(true),
             ByteSize::b(total as usize).to_string(true));

    let err_msg = "Unexpected format of BlockDevices property";
    let bdevs = try!(p.get("BlockDevices"));
    let bdev_vec: &Vec<_> = try!(bdevs.inner()
        .map_err(|_| StratisError::Stratis(InternalError(err_msg.into()))));
    println!("Member devices:");
    for bdev in bdev_vec {
        let inner_vals: &Vec<_> = try!(bdev.inner()
            .map_err(|_| StratisError::Stratis(InternalError(err_msg.into()))));
        let name: &str = try!(inner_vals[0]
            .inner()
            .map_err(|_| StratisError::Stratis(InternalError(err_msg.into()))));
        let status: u32 = try!(inner_vals[1]
            .inner()
            .map_err(|_| StratisError::Stratis(InternalError(err_msg.into()))));
        let status_str = match status {
            0 => "In use",
            1 => "Not in use",
            2 => "Bad",
            3 => "Not present",
            _ => "Unknown",
        };
        println!("{} {}", name, status_str);
    }

    Ok(())
}

fn add(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdevname").unwrap();
    let dev_paths: Vec<_> = args.values_of("devices")
        .unwrap()
        .into_iter()
        .map(|dev| {
            if Path::new(dev).is_absolute() {
                PathBuf::from(dev)
            } else {
                PathBuf::from(format!("/dev/{}", dev))
            }
        })
        .collect();
    let force = args.is_present("force");
    let c = try!(Connection::stratis_connect());
    let fpath = try!(c.stratis_path(name));

    for path in dev_paths {
        let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                             &fpath,
                                             "org.freedesktop.StratisDevice1",
                                             "AddBlockDevice")
            .unwrap();
        m.append_items(&[path.to_string_lossy().into_owned().into(), force.into()]);
        try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));
    }

    Ok(())
}

fn remove(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdevname").unwrap();
    let bd_path = {
        let dev = args.value_of("blockdev").unwrap();
        if Path::new(dev).is_absolute() {
            PathBuf::from(dev)
        } else {
            PathBuf::from(format!("/dev/{}", dev))
        }
    };
    let wipe = args.is_present("wipe");
    let c = try!(Connection::stratis_connect());
    let fpath = try!(c.stratis_path(name));

    let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                         &fpath,
                                         "org.freedesktop.StratisDevice1",
                                         "RemoveBlockDevice")
        .unwrap();
    m.append_items(&[bd_path.to_string_lossy().into_owned().into(), wipe.into()]);
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    Ok(())
}

fn create(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdevname").unwrap();
    let dev_paths: Vec<_> = args.values_of("devices")
        .unwrap()
        .into_iter()
        .map(|dev| {
            if Path::new(dev).is_absolute() {
                PathBuf::from(dev)
            } else {
                PathBuf::from(format!("/dev/{}", dev))
            }
        })
        .map(|pb| pb.to_string_lossy().into_owned().into())
        .collect();
    let force = args.is_present("force");

    let c = try!(Connection::stratis_connect());

    let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                         "/org/freedesktop/Stratis",
                                         "org.freedesktop.StratisService1",
                                         "Create")
        .unwrap();
    m.append_items(&[name.into(), MessageItem::new_array(dev_paths).unwrap(), force.into()]);
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    dbgp!("Stratisdev {} created", name);

    Ok(())
}

fn rename(args: &ArgMatches) -> StratisResult<()> {
    let old_name = args.value_of("Stratisdev_old_name").unwrap();
    let new_name = args.value_of("Stratisdev_new_name").unwrap();

    let c = try!(Connection::stratis_connect());
    let fpath = try!(c.stratis_path(old_name));

    let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                         &fpath,
                                         "org.freedesktop.StratisDevice1",
                                         "SetName")
        .unwrap();
    m.append_items(&[new_name.into()]);
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    dbgp!("Stratisdev name {} changed to {}", old_name, new_name);

    Ok(())
}

fn reshape(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdev").unwrap();

    let c = try!(Connection::stratis_connect());
    let fpath = try!(c.stratis_path(name));

    let m = Message::new_method_call("org.freedesktop.Stratis1",
                                     &fpath,
                                     "org.freedesktop.StratisDevice1",
                                     "Reshape")
        .unwrap();
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    dbgp!("Stratisdev {} starting reshape", name);

    Ok(())
}

fn destroy(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdev").unwrap();

    let c = try!(Connection::stratis_connect());

    let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                         "/org/freedesktop/Stratis",
                                         "org.freedesktop.StratisService1",
                                         "Destroy")
        .unwrap();
    m.append_items(&[name.into()]);
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    dbgp!("Stratisdev {} destroyed", name);

    Ok(())
}

fn teardown(args: &ArgMatches) -> StratisResult<()> {
    let name = args.value_of("Stratisdev").unwrap();

    let c = try!(Connection::stratis_connect());

    let mut m = Message::new_method_call("org.freedesktop.Stratis1",
                                         "/org/freedesktop/Stratis",
                                         "org.freedesktop.StratisService1",
                                         "Teardown")
        .unwrap();
    m.append_items(&[name.into()]);
    try!(c.send_with_reply_and_block(m, DBUS_TIMEOUT));

    dbgp!("Stratisdev {} torn down", name);

    Ok(())
}

fn dbus_server(engine: Rc<RefCell<Engine>>) -> StratisResult<()> {

    let c = try!(Connection::stratis_connect());

    // We can't change a tree from within the tree. So instead
    // register two trees, one with Create and Destroy and another for
    // querying/changing active Stratisdevs/

    let base_tree = try!(dbus_api::get_base_tree(&c, engine));

    // TODO: event loop needs to handle dbus and also dm events (or polling)
    // so we can extend/reshape/delay/whatever in a timely fashion
    let mut last_time = Timespec::new(0, 0);
    for _ in base_tree.run(&c, c.iter(1000)) {
    }
    println!("should never get here");
    for c_item in c.iter(10000) {
        if let ConnectionItem::MethodCall(ref msg) = c_item {
            if msg.msg_type() != MessageType::MethodCall {
                continue;
            }

            base_tree.handle(msg);

        }

        let now = time::now().to_timespec();
        if now < last_time + Duration::seconds(30) {
            continue;
        }

        last_time = now;


    }

    Ok(())
}

fn write_err(err: StratisError) -> StratisResult<()> {
    let mut out = term::stderr().expect("could not get stderr");

    try!(out.fg(term::color::RED));
    try!(writeln!(out, "{}", err.description()));
    try!(out.reset());
    Ok(())
}

fn main() {

    let engine = Rc::new(RefCell::new(SimEngine::new()));
    // TODO: add cmdline option to specify engine

    let r = dbus_server(engine);

    if let Err(r) = r {
        if let Err(e) = write_err(r) {
            panic!("Unable to write to stderr: {}", e)
        }

        exit(1);
    }
}
