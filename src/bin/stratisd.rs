// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    process::exit,
    str::FromStr,
};

use clap::{Arg, ArgAction, Command};
use env_logger::Builder;
use libc::pid_t;
use log::LevelFilter;
use nix::{
    errno::Errno,
    fcntl::{Flock, FlockArg},
    sys::signal::{kill, Signal},
    unistd::{getpid, Pid},
};

use stratisd::stratis::{run, StratisError, StratisResult, VERSION};

const STRATISD_PID_PATH: &str = "/run/stratisd.pid";
const STRATISD_MIN_PID_PATH: &str = "/run/stratisd-min.pid";

/// Configure and initialize the logger.
/// If optional log_level argument is set, use that to set the log level
/// for both stratisd and libstratis. Otherwise, read log configuration
/// parameters from the environment if RUST_LOG is set. Otherwise, just
/// accept the default configuration, which is to log at the severity of
/// error only.
fn initialize_log(log_level: Option<&str>) {
    let mut builder = Builder::new();

    if let Some(log_level) = log_level {
        builder.filter(
            Some("stratisd"),
            LevelFilter::from_str(log_level)
                .expect("argument parser only accepts valid log levels"),
        );
    } else if let Ok(s) = env::var("RUST_LOG") {
        builder.parse_filters(&s);
    }

    builder.init()
}

/// To ensure only one instance of stratisd runs at a time, acquire an
/// exclusive lock. Return an error if lock attempt fails.
fn trylock_pid_file() -> StratisResult<Flock<File>> {
    #[allow(clippy::suspicious_open_options)]
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(STRATISD_PID_PATH)
        .map_err(|err| {
            StratisError::Chained(
                format!("Failed to create or open the stratisd PID file at {STRATISD_PID_PATH}"),
                Box::new(StratisError::from(err)),
            )
        })?;
    let stratisd_file = match Flock::lock(f, FlockArg::LockExclusiveNonblock) {
        Ok(mut f) => {
            f.set_len(0)?;
            f.write_all(getpid().to_string().as_bytes())?;
            Ok(f)
        }
        Err((mut f, _)) => {
            let mut buf = String::new();

            if f.read_to_string(&mut buf).is_err() {
                buf = "<unreadable>".to_string();
            }

            Err(StratisError::Msg(format!(
                "Daemon already running with supposed pid: {buf}"
            )))
        }
    };

    #[allow(clippy::suspicious_open_options)]
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(STRATISD_MIN_PID_PATH)
        .map_err(|err| {
            StratisError::Chained(
                format!(
                    "Failed to create or open the stratisd-min PID file at {STRATISD_MIN_PID_PATH}"
                ),
                Box::new(StratisError::from(err)),
            )
        })?;

    if let Err((mut f, Errno::EWOULDBLOCK)) = Flock::lock(f, FlockArg::LockExclusiveNonblock) {
        let mut string = String::new();
        f.read_to_string(&mut string)?;
        let pid = string
            .parse::<pid_t>()
            .map_err(|_| StratisError::Msg(format!("Failed to parse {} as PID", string)))?;
        kill(Pid::from_raw(pid), Signal::SIGINT)?;

        match Flock::lock(f, FlockArg::LockExclusive) {
            Ok(_) => (),
            Err((_, e)) => {
                return Err(StratisError::Chained(
                    "Failed to wait on stratisd-min to exit".to_string(),
                    Box::new(StratisError::from(e)),
                ))
            }
        };
    }

    stratisd_file
}

fn parse_args() -> Command {
    Command::new("stratisd")
        .version(VERSION)
        .about("Stratis storage management")
        .arg(
            Arg::new("sim")
                .action(ArgAction::SetTrue)
                .long("sim")
                .help("Use simulator engine"),
        )
        .arg(
            Arg::new("log-level")
                .value_parser(["trace", "debug", "info", "warn", "error"])
                .long("log-level")
                .help("Sets level for generation of log messages."),
        )
}

fn main() {
    let matches = parse_args().get_matches();

    // Using a let-expression here so that the scope of the lock file
    // is the rest of the block.
    let lock_file = trylock_pid_file();

    let result = {
        match lock_file {
            Err(err) => Err(err),
            Ok(_) => {
                initialize_log(matches.get_one::<String>("log-level").map(|s| s.as_str()));
                run(matches.get_flag("sim"))
            }
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        exit(1);
    } else {
        exit(0);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_stratisd_parse_args() {
        parse_args().debug_assert();
    }
}
