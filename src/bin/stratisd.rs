// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libstratis;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate devicemapper;
extern crate clap;
extern crate nix;
extern crate crc;
extern crate byteorder;
extern crate uuid;
extern crate time;
extern crate dbus;
extern crate term;
extern crate rand;
extern crate serde;

extern crate custom_derive;
extern crate newtype_derive;
extern crate enum_derive;

#[cfg(test)]
extern crate quickcheck;

use std::io::Write;
use std::env;
use std::error::Error;
use std::process::exit;

use clap::{App, Arg};
use log::LogLevelFilter;
use env_logger::LogBuilder;

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

    let mut builder = LogBuilder::new();
    if matches.is_present("debug") {
        builder.filter(Some("stratisd"), LogLevelFilter::Debug);
        builder.filter(Some("libstratis"), LogLevelFilter::Debug);
    } else {
        if let Ok(s) = env::var("RUST_LOG") {
            builder.parse(&s);
        }
    };

    builder.init()
        .expect("This is the first and only initialization of the logger; it must succeed.");

    let engine: Box<Engine> = {
        if matches.is_present("sim") {
            info!("Using SimEngine");
            Box::new(SimEngine::new())
        } else {
            info!("Using StratEngine");
            Box::new(StratEngine::new())
        }
    };

    if let Err(r) = libstratis::dbus_api::run(engine) {
        if let Err(e) = write_err(r) {
            panic!("Unable to write to stderr: {}", e)
        }

        exit(1);
    }
}
