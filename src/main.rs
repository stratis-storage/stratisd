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
#[macro_use]
extern crate enum_derive;

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
use std::rc::Rc;
use std::cell::RefCell;

use types::{StratisResult, StratisError};

use engine::Engine;
use sim_engine::SimEngine;


fn dbus_server(engine: Rc<RefCell<Engine>>) -> StratisResult<()> {

    let result = dbus_api::run(engine);

    result
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
    //  let context = Rc::new(RefCell::new(Context::new()));

    let r = dbus_server(engine);

    if let Err(r) = r {
        if let Err(e) = write_err(r) {
            panic!("Unable to write to stderr: {}", e)
        }

        exit(1);
    }
}
