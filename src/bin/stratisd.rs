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
extern crate libc;

extern crate custom_derive;
extern crate enum_derive;

#[cfg(test)]
extern crate quickcheck;

use std::io::Write;
use std::env;
use std::error::Error;
use std::rc::Rc;
use std::cell::RefCell;

use clap::{App, Arg};
use log::LogLevelFilter;
use env_logger::LogBuilder;
use dbus::WatchEvent;

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

    builder
        .init()
        .expect("This is the first and only initialization of the logger; it must succeed.");

    let engine: Rc<RefCell<Engine>> = {
        if matches.is_present("sim") {
            info!("Using SimEngine");
            Rc::new(RefCell::new(SimEngine::new()))
        } else {
            info!("Using StratEngine");
            Rc::new(RefCell::new(StratEngine::new()))
        }
    };

    let (dbus_conn, mut tree, dbus_context) = libstratis::dbus_api::connect(engine.clone())
        .expect("Could not connect to D-Bus");

    // Get a list of fds to poll for
    let mut fds: Vec<_> = dbus_conn
        .watch_fds()
        .iter()
        .map(|w| w.to_pollfd())
        .collect();

    loop {
        // Poll them with a 10 s timeout
        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::c_ulong, 10000) };
        assert!(r >= 0);

        // And handle incoming events
        for pfd in fds.iter().filter(|pfd| pfd.revents != 0) {
            for item in dbus_conn.watch_handle(pfd.fd, WatchEvent::from_revents(pfd.revents)) {
                if let Err(r) = libstratis::dbus_api::handle(&dbus_conn,
                                                             item,
                                                             &mut tree,
                                                             &dbus_context) {
                    if let Err(e) = write_err(r) {
                        panic!("Unable to write to stderr: {}", e)
                    }
                }
            }
        }

        // Ask the engine to check its pools
        engine.borrow_mut().check()
    }
}
