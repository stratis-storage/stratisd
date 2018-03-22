// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate devicemapper;
extern crate libstratis;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate clap;
#[cfg(feature="dbus_enabled")]
extern crate dbus;
extern crate libc;
extern crate libudev;
extern crate nix;

#[cfg(test)]
extern crate quickcheck;

use std::cell::RefCell;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::process::exit;
use std::rc::Rc;

use clap::{App, Arg};
use env_logger::LogBuilder;
use libc::pid_t;
use log::{LogLevelFilter, SetLoggerError};
use nix::fcntl::{flock, FlockArg};
use nix::unistd::getpid;

#[cfg(feature="dbus_enabled")]
use dbus::WatchEvent;

use devicemapper::Device;

use libstratis::engine::{Engine, SimEngine, StratEngine};
use libstratis::stratis::{StratisError, StratisResult, VERSION};

const STRATISD_PID_PATH: &str = "/var/run/stratisd.pid";

/// If writing a program error to stderr fails, panic.
fn print_err(err: StratisError) -> () {
    eprintln!("{}", err);
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

/// Given a udev event check to see if it's an add and if it is return the device node and
/// devicemapper::Device.
fn handle_udev_add(event: &libudev::Event) -> Option<(PathBuf, Device)> {
    if event.event_type() == libudev::EventType::Add {
        let device = event.device();
        return device
                   .devnode()
                   .and_then(|devnode| {
                                 device
                                     .devnum()
                                     .and_then(|devnum| {
                                                   Some((PathBuf::from(devnode),
                                                         Device::from(devnum)))
                                               })
                             });
    }
    None
}

/// To ensure only one instance of stratisd runs at a time, acquire an
/// exclusive lock. Exit if lock attempt fails.
fn create_pid_file() -> StratisResult<File> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(STRATISD_PID_PATH)?;
    match flock(f.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
        Ok(_) => {
            f.write(format!("{}\n", getpid()).as_bytes())?;
            Ok(f)
        }
        Err(_) => {
            let mut buf = String::new();
            f.read_to_string(&mut buf)?;
            // pidfile is supposed to contain pid of holder. But you never
            // know so be paranoid.
            let pid_str = buf.split_whitespace()
                .next()
                .and_then(|s| s.parse::<pid_t>().ok())
                .map(|pid| format!("{}", pid))
                .unwrap_or_else(|| "<unknown>".into());
            Err(StratisError::Error(format!("Daemon already running with pid: {}", pid_str)))
        }
    }
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

    let _pidfile = create_pid_file()?;

    // We must setup a udev listener before we initialize the
    // engine. It is possible that a device may appear after the engine has read the
    // /dev directory but before it has completed initialization. Unless the udev
    // event has been recorded, the engine will be unaware of the existence of the
    // device.
    // This is especially important since stratisd is required to be able to run
    // during early boot.
    let context = libudev::Context::new()?;
    let mut monitor = libudev::Monitor::new(&context)?;
    monitor.match_subsystem_devtype("block", "disk")?;
    let mut udev = monitor.listen()?;

    let engine: Rc<RefCell<Engine>> = {
        if matches.is_present("sim") {
            info!("Using SimEngine");
            Rc::new(RefCell::new(SimEngine::default()))
        } else {
            info!("Using StratEngine");
            Rc::new(RefCell::new(StratEngine::initialize()?))
        }
    };

    /*
    The file descriptor array indexes are laid out in the following:

    0   == Always udev fd index
    1   == engine index if eventable
    1/2 == Start of dbus client file descriptor(s), 1 if engine is not eventable, else 2
    */
    const FD_INDEX_UDEV: usize = 0;
    const FD_INDEX_ENGINE: usize = 1;


    let mut fds = Vec::new();

    fds.push(libc::pollfd {
                 fd: udev.as_raw_fd(),
                 revents: 0,
                 events: libc::POLLIN,
             });

    let eventable = engine.borrow().get_eventable()?;

    // The variable _dbus_client_index_start is only used when dbus support is compiled in, thus
    // we denote the value as not needed to compile when dbus support is not included.
    let (engine_eventable, poll_timeout, _dbus_client_index_start) = match eventable {
        Some(ref evt) => {
            fds.push(libc::pollfd {
                         fd: evt.get_pollable_fd(),
                         revents: 0,
                         events: libc::POLLIN,
                     });

            // Don't timeout if eventable, we are event driven
            (true, -1, FD_INDEX_ENGINE + 1)
        }

        // We periodically need to timeout as we are not event driven
        None => (false, 10000, FD_INDEX_ENGINE),
    };

    #[cfg(feature="dbus_enabled")]
    let (mut dbus_conn, mut tree, base_object_path, mut dbus_context) =
        libstratis::dbus_api::connect(Rc::clone(&engine))?;

    loop {
        // Process any udev block events
        if fds[FD_INDEX_UDEV].revents != 0 {
            loop {
                match udev.receive_event() {
                    Some(event) => {
                        if let Some((devnode, device)) = handle_udev_add(&event) {

                            // If block evaluate returns an error we are going to ignore it as
                            // there is nothing we can do for a device we are getting errors with.
                            let pool_uuid = engine
                                .borrow_mut()
                                .block_evaluate(devnode, device)
                                .unwrap_or(None);

                            // We need to pretend that we aren't using the variable _pool_uuid so
                            // that we can conditionally compile out the register_pool when dbus
                            // is not enabled.
                            if let Some(_pool_uuid) = pool_uuid {
                                #[cfg(feature="dbus_enabled")]
                                libstratis::dbus_api::register_pool(&mut dbus_conn,
                                                                    Rc::clone(&engine),
                                                                    &mut dbus_context,
                                                                    &mut tree,
                                                                    _pool_uuid,
                                                                    &base_object_path)?;
                            }
                        }
                    }
                    None => {
                        break;
                    }
                };
            }
        }

        // Handle engine events, if the engine is eventable
        if engine_eventable {
            if fds[FD_INDEX_ENGINE].revents != 0 {
                fds[FD_INDEX_ENGINE].revents = 0;

                eventable
                    .as_ref()
                    .expect("eventable.is_some()")
                    .clear_event()?;

                engine.borrow_mut().evented()?;
            }
        } else {
            // Unconditionally call evented() if engine has no eventable.
            // This looks like a bad idea, but the only engine that has
            // no eventable is the sim engine, and for that engine, evented()
            // is essentially a no-op.
            engine.borrow_mut().evented()?;
        }

        // Iterate through D-Bus file descriptors (if enabled)
        #[cfg(feature="dbus_enabled")]
        {
            for pfd in fds[_dbus_client_index_start..]
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
            fds.truncate(_dbus_client_index_start);

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
