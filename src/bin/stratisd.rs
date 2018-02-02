// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate libstratis;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate clap;
#[cfg(feature="dbus_enabled")]
extern crate dbus;
extern crate libc;

#[cfg(test)]
extern crate quickcheck;

use std::env;
use std::error::Error;
use std::rc::Rc;
use std::cell::RefCell;
use std::process::exit;

use clap::{App, Arg};
use env_logger::LogBuilder;
use log::{LogLevelFilter, SetLoggerError};

#[cfg(feature="dbus_enabled")]
use dbus::WatchEvent;

use libstratis::engine::{Engine, SimEngine, StratEngine};
use libstratis::stratis::{StratisResult, StratisError, VERSION};

/// If writing a program error to stderr fails, panic.
fn print_err(err: StratisError) -> () {
    eprintln!("{}", err.description());
}

/// Configure and initialize the logger.
/// If debug is true, log at debug level. Otherwise read log configuration
/// parameters from the environment if RUST_LOG is set. Otherwise, just
/// accept the default configuration.
fn initialize_log(debug: bool) -> Result<(), SetLoggerError> {
    let mut builder = LogBuilder::new();
    if debug {
        builder.filter(Some("stratisd"), LogLevelFilter::Debug);
        builder.filter(Some("libstratis"), LogLevelFilter::Debug);
    } else {
        if let Ok(s) = env::var("RUST_LOG") {
            builder.parse(&s);
        }
    };

    builder.init()
}

fn run() -> StratisResult<()> {

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

    initialize_log(matches.is_present("debug"))
        .expect("This is the first and only invocation of this method; it must succeed.");

    let engine: Rc<RefCell<Engine>> = {
        if matches.is_present("sim") {
            info!("Using SimEngine");
            Rc::new(RefCell::new(SimEngine::default()))
        } else {
            info!("Using StratEngine");
            Rc::new(RefCell::new(StratEngine::initialize()?))
        }
    };

    #[cfg(feature="dbus_enabled")]
    let (dbus_conn, mut tree, dbus_context) = libstratis::dbus_api::connect(Rc::clone(&engine))?;

    let mut fds = Vec::new();

    let mut eventable = engine.borrow_mut().get_eventable()?;
    let engine_fds_end_idx = match eventable {
        Some(ref mut evt) => {
            fds.push(libc::pollfd {
                         fd: evt.get_pollable_fd(),
                         revents: 0,
                         events: libc::POLLIN,
                     });
            1
        }
        None => 0,
    };

    // Don't timeout if eventable lets us be event-driven
    let poll_timeout = if engine_fds_end_idx != 0 { -1 } else { 10000 };

    loop {
        // Unconditionally call evented() if engine has no eventable.
        // This looks like a bad idea, but the only engine that has
        // no eventable is the sim engine, and for that engine, evented()
        // is essentially a no-op.
        if engine_fds_end_idx == 0 {
            engine.borrow_mut().evented()?;
        }

        // Handle engine fd, if there is one.
        // If engine has no eventable, there is no engine fd; in that case
        // the body of the loop is executed zero times.
        for pfd in fds[..engine_fds_end_idx]
                .iter_mut()
                .filter(|pfd| pfd.revents != 0) {
            pfd.revents = 0;
            eventable
                .as_mut()
                .expect("index < engine_fds_end_idx <=> index == 0 <=> eventable.is_some()")
                .clear_event()?;
            engine.borrow_mut().evented()?;
        }

        // Iterate through D-Bus file descriptors (if enabled)
        #[cfg(feature="dbus_enabled")]
        {
            for pfd in fds[engine_fds_end_idx..]
                    .iter()
                    .filter(|pfd| pfd.revents != 0) {
                for item in dbus_conn.watch_handle(pfd.fd, WatchEvent::from_revents(pfd.revents)) {
                    if let Err(r) = libstratis::dbus_api::handle(&dbus_conn,
                                                                 &item,
                                                                 &mut tree,
                                                                 &dbus_context) {
                        print_err(From::from(r));
                    }
                }
            }

            // Refresh list of dbus fds to poll for every time. This can change as
            // D-Bus clients come and go.
            fds.truncate(engine_fds_end_idx);

            fds.extend(dbus_conn.watch_fds().iter().map(|w| w.to_pollfd()));
        }

        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::c_ulong, poll_timeout) };
        assert!(r >= 0);
    }
}

fn main() {
    let error_code = match run() {
        Ok(_) => 0,
        Err(err) => {
            print_err(err);
            1
        }
    };
    exit(error_code);
}
