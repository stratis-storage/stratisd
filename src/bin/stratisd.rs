// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
extern crate libstratis;

extern crate devicemapper;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate bidir_map;
extern crate byteorder;
extern crate uuid;
extern crate time;
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

#[cfg(test)]
extern crate quickcheck;

use std::io::Write;
use std::error::Error;
use std::process::exit;

use clap::{App, Arg};

use libstratis::engine::Engine;
use libstratis::engine::sim_engine::SimEngine;
use libstratis::engine::strat_engine::StratEngine;
use libstratis::stratis::VERSION;
use libstratis::types::{StratisResult, StratisError};

fn write_err(err: StratisError) -> StratisResult<()> {
    let mut out = term::stderr().expect("could not get stderr");

    try!(out.fg(term::color::RED));
    try!(writeln!(out, "{}", err.description()));
    try!(out.reset());
    Ok(())
}

pub static mut DEBUG: bool = false;

macro_rules! dbgp {
    ($($arg:tt)*) => (
        unsafe {
            if DEBUG {
                println!($($arg)*)
            }
        })
}

fn main() {
    let matches = App::new("stratis")
        .version(VERSION)
        .about("Stratis storage management")
        .arg(Arg::with_name("debug")
            .long("debug")
            .help("Print additional output for debugging"))
        .arg(Arg::with_name("sim")
            .long("sim")
            .help("Use simulator engine"))
        .get_matches();

    if matches.is_present("debug") {
        unsafe { DEBUG = true }
    };

    let engine: Box<Engine> = {
        if matches.is_present("sim") {
            dbgp!("Using SimEngine");
            Box::new(SimEngine::new())
        } else {
            dbgp!("Using StratEngine");
            Box::new(StratEngine::new())
        }
    };

    let r = libstratis::dbus_api::run(engine);

    if let Err(r) = r {
        if let Err(e) = write_err(r) {
            panic!("Unable to write to stderr: {}", e)
        }

        exit(1);
    }
}
