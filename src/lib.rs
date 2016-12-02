// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(dead_code)] // only temporary, until more stuff is filled in

extern crate devicemapper;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate bidir_map;
extern crate byteorder;
extern crate uuid;
extern crate time;
extern crate bytesize;
extern crate dbus;
extern crate term;
extern crate rand;
extern crate serde;

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


mod consts;
mod dbus_api;
mod dbus_consts;
mod engine;
mod sim_engine;
mod strat_engine;
mod stratis;
mod types;

use std::error::Error;

/// Choose the engine, simulator or real.
pub fn get_engine(sim: bool) -> Box<engine::Engine> {
    if sim {
        dbgp!("Using SimEngine");
        Box::new(sim_engine::SimEngine::new())
    } else {
        dbgp!("Using StratEngine");
        Box::new(strat_engine::StratEngine::new())
    }
}

/// Set debug
pub fn set_debug(debug_state: bool) -> () {
    unsafe { debug = debug_state }
}


/// Display stratis error
pub fn write_err(err: types::StratisError) -> types::StratisResult<()> {
    let mut out = term::stderr().expect("could not get stderr");

    try!(out.fg(term::color::RED));
    try!(writeln!(out, "{}", err.description()));
    try!(out.reset());
    Ok(())
}

/// Start stratisd D-Bus service
pub fn run(engine: Box<engine::Engine>) -> types::StratisResult<()> {
    dbus_api::run(engine)
}
