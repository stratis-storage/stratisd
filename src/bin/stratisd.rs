// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![cfg_attr(not(feature = "clippy"), allow(unknown_lints))]
#![allow(doc_markdown)]

extern crate devicemapper;
extern crate libstratis;
#[macro_use]
extern crate log;
extern crate chrono;
extern crate clap;
#[cfg(feature = "dbus_enabled")]
extern crate dbus;
extern crate env_logger;
extern crate libc;
extern crate libudev;
extern crate nix;

use std::cell::RefCell;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::process::exit;
use std::rc::Rc;

use chrono::Duration;
use clap::{App, Arg, ArgMatches};
use env_logger::Builder;
use libc::pid_t;
use log::LevelFilter;
use nix::fcntl::{flock, FlockArg};
use nix::sys::signal::{self, SigSet};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use nix::unistd::getpid;

#[cfg(feature = "dbus_enabled")]
use dbus::WatchEvent;

use devicemapper::Device;

use libstratis::engine::{Engine, SimEngine, StratEngine};
use libstratis::stratis::{alarm, buff_log};
use libstratis::stratis::{StratisError, StratisResult, VERSION};

const STRATISD_PID_PATH: &str = "/var/run/stratisd.pid";

/// Interval at which to have stratisd perform periodic tasks.
const STRATISD_ALARM_SECONDS: u32 = 600;

/// Number of minutes to buffer log entries.
const DEFAULT_LOG_HOLD_MINUTES: i64 = 30;

/// If writing a program error to stderr fails, panic.
fn print_err(err: &StratisError) -> () {
    eprintln!("{}", err);
}

/// Log the engine state in a formatted way.
fn log_engine_state(engine: &Engine) {
    debug!("Engine state: \n{:#?}", engine);
}

/// Configure the env_logger as necessary in order to allow the buffered
/// logger to work correctly. Return a Handle to the underlying env_logger.
pub fn from_env_logger(
    mut builder: env_logger::Builder,
    pass_through: bool,
    hold_time: Option<Duration>,
) -> buff_log::Handle<env_logger::Logger> {
    // Do not have the env_logger set the timestamp. Because the entries are
    // buffered, the timestamp set by the env_logger will correspond to the
    // time at which the entry was dumped, not the time of its origination.
    builder.default_format_timestamp(false);
    buff_log::Logger::new(builder.build(), pass_through, hold_time).init()
}

/// Configure and initialize the logger.
/// If debug is true, log at debug level. Otherwise read log configuration
/// parameters from the environment if RUST_LOG is set. Otherwise, just
/// accept the default configuration.
fn initialize_log(debug: bool) -> buff_log::Handle<env_logger::Logger> {
    let mut builder = Builder::new();
    if debug {
        builder.filter(Some("stratisd"), LevelFilter::Debug);
        builder.filter(Some("libstratis"), LevelFilter::Debug);
        from_env_logger(builder, true, None)
    } else {
        builder.filter_level(LevelFilter::Trace);
        if let Ok(s) = env::var("RUST_LOG") {
            builder.parse(&s);
        }
        from_env_logger(
            builder,
            false,
            Some(Duration::minutes(DEFAULT_LOG_HOLD_MINUTES)),
        )
    }
}

/// Given a udev event check to see if it's an add and if it is return the device node and
/// devicemapper::Device.
fn handle_udev_add(event: &libudev::Event) -> Option<(Device, PathBuf)> {
    if event.event_type() == libudev::EventType::Add {
        let device = event.device();
        return device.devnode().and_then(|devnode| {
            device
                .devnum()
                .and_then(|devnum| Some((Device::from(devnum), PathBuf::from(devnode))))
        });
    }
    None
}

/// To ensure only one instance of stratisd runs at a time, acquire an
/// exclusive lock. Return an error if lock attempt fails.
fn trylock_pid_file() -> StratisResult<File> {
    let mut f = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(STRATISD_PID_PATH)
    {
        Ok(f) => f,
        Err(e) => {
            if e.kind() == ErrorKind::PermissionDenied {
                return Err(StratisError::Error(
                    "Must be running as root in order to start daemon.".to_string(),
                ));
            }
            return Err(e.into());
        }
    };
    match flock(f.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
        Ok(_) => {
            f.write_all(format!("{}\n", getpid()).as_bytes())?;
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
            Err(StratisError::Error(format!(
                "Daemon already running with pid: {}",
                pid_str
            )))
        }
    }
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered. Dump log entries on specified signal
/// via buff_log.
fn run(matches: &ArgMatches, buff_log: &buff_log::Handle<env_logger::Logger>) -> StratisResult<()> {
    // Ensure that the debug log is output when we leave this function.
    let _guard = buff_log.to_guard();

    // Setup a udev listener before initializing the engine. A device may
    // appear after the engine has read the /dev directory but before it has
    // completed initialization. Unless the udev event has been recorded, the
    // engine will miss the device.
    // This is especially important since stratisd must run during early boot.
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
    The file descriptor array indexes are:

    0   == Always udev fd index
    1   == SIGNAL FD index
    2   == engine index if eventable
    2/3 == Start of dbus client file descriptor(s)
            * 2 if engine is not eventable
            * else 3
    */
    const FD_INDEX_UDEV: usize = 0;
    const FD_INDEX_SIGNALFD: usize = 1;
    const FD_INDEX_ENGINE: usize = 2;

    /*
    fds is a Vec of libc::pollfd structs. Ideally, it would be possible
    to use the higher level nix crate to handle polling. If this were possible,
    then the Vec would be one of nix::poll::PollFds and this would be more
    rustic. Unfortunately, the rust D-Bus library requires an explicit file
    descriptor to be passed as an argument to Connection::watch_handle(),
    and the explicit file descriptor can not be extracted from the PollFd
    struct. So, at this time, sticking with libc is less complex than
    converting to using nix, because if using nix, the file descriptor would
    have to be maintained in the Vec as well as the PollFd struct.
    */
    let mut fds = Vec::new();

    fds.push(libc::pollfd {
        fd: udev.as_raw_fd(),
        revents: 0,
        events: libc::POLLIN,
    });

    // Signals can be queued up on this file descriptor
    let mut sfd = {
        let mut mask = SigSet::empty();
        mask.add(signal::SIGINT);
        mask.add(signal::SIGALRM);
        mask.add(signal::SIGUSR1);
        mask.thread_block()?;
        SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK)?
    };

    fds.push(libc::pollfd {
        fd: sfd.as_raw_fd(),
        revents: 0,
        events: libc::POLLIN,
    });

    let eventable = engine.borrow().get_eventable();

    let poll_timeout = match eventable {
        Some(ref evt) => {
            fds.push(libc::pollfd {
                fd: evt.get_pollable_fd(),
                revents: 0,
                events: libc::POLLIN,
            });
            -1
        }
        None => 10000,
    };

    #[cfg(feature = "dbus_enabled")]
    let dbus_client_index_start = if eventable.is_some() {
        FD_INDEX_ENGINE + 1
    } else {
        FD_INDEX_ENGINE
    };

    #[cfg(feature = "dbus_enabled")]
    let (dbus_conn, mut tree, base_object_path, dbus_context) =
        libstratis::dbus_api::connect(Rc::clone(&engine))?;

    #[cfg(feature = "dbus_enabled")]
    for (_, pool_uuid, pool) in engine.borrow().pools() {
        libstratis::dbus_api::register_pool(
            &dbus_conn,
            &dbus_context,
            &mut tree,
            pool_uuid,
            pool,
            &base_object_path,
        )?;
    }

    log_engine_state(&*engine.borrow());
    alarm(STRATISD_ALARM_SECONDS);

    loop {
        // Process any udev block events
        if fds[FD_INDEX_UDEV].revents != 0 {
            while let Some(event) = udev.receive_event() {
                if let Some((device, devnode)) = handle_udev_add(&event) {
                    // If block evaluate returns an error we are going to ignore it as
                    // there is nothing we can do for a device we are getting errors with.
                    #[cfg(not(feature = "dbus_enabled"))]
                    let _ = engine.borrow_mut().block_evaluate(device, devnode);

                    #[cfg(feature = "dbus_enabled")]
                    {
                        let pool_uuid = engine
                            .borrow_mut()
                            .block_evaluate(device, devnode)
                            .unwrap_or(None);

                        if let Some(pool_uuid) = pool_uuid {
                            libstratis::dbus_api::register_pool(
                                &dbus_conn,
                                &dbus_context,
                                &mut tree,
                                pool_uuid,
                                engine
                                    .borrow()
                                    .get_pool(pool_uuid)
                                    .expect(
                                        "block_evaluate() returned a pool UUID, pool must be available",
                                    )
                                    .1,
                                &base_object_path,
                            )?;
                        }
                    }
                }
            }
        }

        // Process any signals off signalfd
        if fds[FD_INDEX_SIGNALFD].revents != 0 {
            match sfd.read_signal() {
                // This is an unsafe conversion, but in this context that is
                // mostly harmless. A negative converted value, which is
                // virtually impossible, will not match any of the masked
                // values, and stratisd will panic and exit.
                Ok(Some(sig)) => match sig.ssi_signo as i32 {
                    nix::libc::SIGALRM => {
                        info!("SIGALRM received, performing periodic tasks");
                        log_engine_state(&*engine.borrow());
                        alarm(STRATISD_ALARM_SECONDS);
                    }
                    nix::libc::SIGUSR1 => {
                        info!(
                            "SIGUSR1 received, dumping {} buffered log entries",
                            buff_log.buffered_count()
                        );
                        buff_log.dump()
                    }
                    nix::libc::SIGINT => {
                        info!("SIGINT received, exiting");
                        return Ok(());
                    }
                    signo => {
                        panic!("Caught an impossible signal {:?}", signo);
                    }
                },
                // No signals waiting (SFD_NONBLOCK flag is set)
                Ok(None) => (),

                // Pessimistically exit on an error reading the signal.
                Err(err) => return Err(err.into()),
            }
        }

        // Handle engine events, if the engine is eventable
        match eventable {
            Some(ref evt) => {
                if fds[FD_INDEX_ENGINE].revents != 0 {
                    evt.clear_event()?;
                    engine.borrow_mut().evented()?;
                }
            }
            None => {
                // Unconditionally call evented() if engine has no eventable.
                // This looks like a bad idea, but the only engine that has
                // no eventable is the sim engine, and for that engine,
                // evented() is essentially a no-op.
                engine.borrow_mut().evented()?;
            }
        }

        // Iterate through D-Bus file descriptors (if enabled)
        #[cfg(feature = "dbus_enabled")]
        {
            for pfd in fds[dbus_client_index_start..]
                .iter()
                .filter(|pfd| pfd.revents != 0)
            {
                for item in dbus_conn.watch_handle(pfd.fd, WatchEvent::from_revents(pfd.revents)) {
                    if let Err(r) =
                        libstratis::dbus_api::handle(&dbus_conn, &item, &mut tree, &dbus_context)
                    {
                        log_engine_state(&*engine.borrow());
                        print_err(&From::from(r));
                    }
                }
            }

            // Refresh list of dbus fds to poll for every time. This can change as
            // D-Bus clients come and go.
            fds.truncate(dbus_client_index_start);

            fds.extend(dbus_conn.watch_fds().iter().map(|w| w.to_pollfd()));
        }

        let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::c_ulong, poll_timeout) };

        // TODO: refine this behavior.
        // Different behaviors may be indicated, depending on the value of
        // errno when return value is -1.
        if r < 0 {
            return Err(StratisError::Error(format!(
                "poll command failed: number of fds: {}, timeout: {}",
                fds.len(),
                poll_timeout
            )));
        }
    }
}

fn main() {
    let matches = App::new("stratis")
        .version(VERSION)
        .about("Stratis storage management")
        .arg(
            Arg::with_name("debug")
                .long("debug")
                .help("Print additional output for debugging"),
        )
        .arg(
            Arg::with_name("sim")
                .long("sim")
                .help("Use simulator engine"),
        )
        .get_matches();

    // Using a let-expression here so that the scope of the lock file
    // is the rest of the block.
    let lock_file = trylock_pid_file();

    let result = {
        match lock_file {
            Err(err) => Err(err),
            Ok(_) => {
                let log_handle = initialize_log(matches.is_present("debug"));
                run(&matches, &log_handle)
            }
        }
    };

    if let Err(err) = result {
        print_err(&err);
        exit(1);
    } else {
        exit(0);
    }
}
