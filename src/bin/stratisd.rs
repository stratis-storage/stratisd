// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(clippy::doc_markdown)]

#[macro_use]
extern crate log;

use std::{
    cell::RefCell,
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::io::{AsRawFd, RawFd},
    process::exit,
    rc::Rc,
};

use chrono::Duration;
use clap::{App, Arg, ArgMatches};
use env_logger::Builder;
use libc::pid_t;
use log::LevelFilter;
use nix::{
    fcntl::{flock, FlockArg},
    sys::{
        signal::{self, SigSet},
        signalfd::{SfdFlags, SignalFd},
    },
    unistd::getpid,
};

use libstratis::{
    engine::{Engine, SimEngine, StratEngine},
    stratis::{buff_log, MaybeDbusSupport, StratisError, StratisResult, VERSION},
};

const STRATISD_PID_PATH: &str = "/run/stratisd.pid";

/// Number of minutes to buffer log entries.
const DEFAULT_LOG_HOLD_MINUTES: i64 = 30;

/// If writing a program error to stderr fails, panic.
fn print_err(err: &StratisError) {
    eprintln!("{}", err);
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

/// To ensure only one instance of stratisd runs at a time, acquire an
/// exclusive lock. Return an error if lock attempt fails.
fn trylock_pid_file() -> StratisResult<File> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(STRATISD_PID_PATH)
        .map_err(|err| {
            StratisError::Error(format!(
                "Failed to create or open the stratisd PID file at {}: {}",
                STRATISD_PID_PATH, err
            ))
        })?;
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
            let pid_str = buf
                .split_whitespace()
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

// A facility for listening for and handling udev events that stratisd
// considers interesting.
struct UdevMonitor<'a> {
    socket: libudev::MonitorSocket<'a>,
}

impl<'a> UdevMonitor<'a> {
    fn create(context: &'a libudev::Context) -> StratisResult<UdevMonitor<'a>> {
        let mut monitor = libudev::Monitor::new(context)?;
        monitor.match_subsystem("block")?;

        Ok(UdevMonitor {
            socket: monitor.listen()?,
        })
    }

    fn as_raw_fd(&mut self) -> RawFd {
        self.socket.as_raw_fd()
    }

    /// Handle udev events.
    /// Check if a pool can be constructed and update engine and D-Bus layer
    /// data structures if so.
    fn handle_events(&mut self, engine: &mut dyn Engine, dbus_support: &mut MaybeDbusSupport) {
        while let Some(event) = self.socket.receive_event() {
            if let Some((pool_uuid, pool)) = engine.handle_event(&event) {
                dbus_support.register_pool(pool_uuid, pool);
            }
        }
    }
}

// Process any pending signals, return true if SIGINT received.
// Return an error if there was an error reading the signal.
fn process_signal(
    sfd: &mut SignalFd,
    buff_log: &buff_log::Handle<env_logger::Logger>,
) -> StratisResult<bool> {
    match sfd.read_signal() {
        // This is an unsafe conversion, but in this context that is
        // mostly harmless. A negative converted value, which is
        // virtually impossible, will not match any of the masked
        // values, and stratisd will panic and exit.
        Ok(Some(sig)) => match sig.ssi_signo as i32 {
            nix::libc::SIGUSR1 => {
                info!(
                    "SIGUSR1 received, dumping {} buffered log entries",
                    buff_log.buffered_count()
                );
                buff_log.dump();
                Ok(false)
            }
            nix::libc::SIGINT => {
                info!("SIGINT received, exiting");
                Ok(true)
            }
            signo => {
                panic!("Caught an impossible signal {:?}", signo);
            }
        },
        // No signals waiting (SFD_NONBLOCK flag is set)
        Ok(None) => Ok(false),

        Err(err) => Err(err.into()),
    }
}

/// Handle blocking the event loop
fn process_poll(poll_timeout: i32, fds: &mut Vec<libc::pollfd>) -> StratisResult<()> {
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
    Ok(())
}

/// Set up all sorts of signal and event handling mechanisms.
/// Initialize the engine and keep it running until a signal is received
/// or a fatal error is encountered. Dump log entries on specified signal
/// via buff_log.
fn run(matches: &ArgMatches, buff_log: &buff_log::Handle<env_logger::Logger>) -> StratisResult<()> {
    // Ensure that the debug log is output when we leave this function.
    let _guard = buff_log.to_guard();

    let mut dbus_support = MaybeDbusSupport::new();

    // Setup a udev listener before initializing the engine. A device may
    // appear after the engine has processed the udev db, but before it has
    // completed initialization. Unless the udev event has been recorded, the
    // engine will miss the device.
    // This is especially important since stratisd must run during early boot.
    let context = libudev::Context::new()?;
    let mut udev_monitor = UdevMonitor::create(&context)?;

    let engine: Rc<RefCell<dyn Engine>> = {
        info!("stratis daemon version {} started", VERSION);
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
        fd: udev_monitor.as_raw_fd(),
        revents: 0,
        events: libc::POLLIN,
    });

    // Signals can be queued up on this file descriptor
    let mut sfd = {
        let mut mask = SigSet::empty();
        mask.add(signal::SIGINT);
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

    if let Some(evt) = eventable {
        fds.push(libc::pollfd {
            fd: evt.get_pollable_fd(),
            revents: 0,
            events: libc::POLLIN,
        });
    };

    let dbus_client_index_start = if eventable.is_some() {
        FD_INDEX_ENGINE + 1
    } else {
        FD_INDEX_ENGINE
    };

    loop {
        if fds[FD_INDEX_UDEV].revents != 0 {
            udev_monitor.handle_events(&mut *engine.borrow_mut(), &mut dbus_support)
        }

        if fds[FD_INDEX_SIGNALFD].revents != 0 {
            match process_signal(&mut sfd, buff_log) {
                Ok(should_exit) => {
                    if should_exit {
                        return Ok(());
                    }
                }
                Err(e) => return Err(e),
            }
        }

        if let Some(evt) = eventable {
            if fds[FD_INDEX_ENGINE].revents != 0 {
                evt.clear_event()?;
                engine.borrow_mut().evented()?;
            }
        }

        dbus_support.process(&engine, &mut fds, dbus_client_index_start);

        process_poll(dbus_support.poll_timeout(), &mut fds)?;
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
