// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![allow(clippy::doc_markdown)]

use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::io::AsRawFd,
    process::exit,
};

use chrono::Duration;
use clap::{App, Arg};
use env_logger::Builder;
use log::LevelFilter;
use nix::{
    fcntl::{flock, FlockArg},
    unistd::getpid,
};

use libstratis::stratis::{buff_log, run, StratisError, StratisResult, VERSION};

const STRATISD_PID_PATH: &str = "/run/stratisd.pid";

/// Number of minutes to buffer log entries.
const DEFAULT_LOG_HOLD_MINUTES: i64 = 30;

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
            f.write_all(getpid().to_string().as_bytes())?;
            Ok(f)
        }
        Err(_) => {
            let mut buf = String::new();

            if f.read_to_string(&mut buf).is_err() {
                buf = "<unreadable>".to_string();
            }

            Err(StratisError::Error(format!(
                "Daemon already running with supposed pid: {}",
                buf
            )))
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
                run(matches.is_present("sim"), &log_handle)
            }
        }
    };

    if let Err(err) = result {
        eprintln!("{}", err);
        exit(1);
    } else {
        exit(0);
    }
}
