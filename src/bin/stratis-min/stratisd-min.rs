// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::io::AsRawFd,
    str::FromStr,
};

use clap::{Arg, Command};
use env_logger::Builder;
use log::LevelFilter;
use nix::{
    fcntl::{flock, FlockArg},
    unistd::getpid,
};

use stratisd::stratis::{run, StratisError, StratisResult, VERSION};

const STRATISD_PID_PATH: &str = "/run/stratisd.pid";
const STRATISD_MIN_PID_PATH: &str = "/run/stratisd-min.pid";

fn parse_args() -> Command {
    Command::new("stratisd-min")
        .version(VERSION)
        .arg(
            Arg::new("log_level")
                .value_parser(["trace", "debug", "info", "warn", "error"])
                .long("log-level")
                .help("Sets level for generation of log messages."),
        )
        .arg(
            Arg::new("sim")
                .long("sim")
                .num_args(0)
                .help("Enables sim engine."),
        )
}

/// To ensure only one instance of stratisd runs at a time, acquire an
/// exclusive lock. Return an error if lock attempt fails.
fn trylock_pid_file() -> StratisResult<File> {
    #[allow(unknown_lints)]
    #[allow(clippy::suspicious_open_options)]
    let mut f = OpenOptions::new()
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
    let stratisd_min_file = match flock(f.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
        Ok(_) => {
            f.write_all(getpid().to_string().as_bytes())?;
            f
        }
        Err(_) => {
            let mut buf = String::new();

            if f.read_to_string(&mut buf).is_err() {
                buf = "<unreadable>".to_string();
            }

            return Err(StratisError::Msg(format!(
                "Daemon already running with supposed pid: {buf}"
            )));
        }
    };

    #[allow(unknown_lints)]
    #[allow(clippy::suspicious_open_options)]
    let mut f = OpenOptions::new()
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
    match flock(f.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
        Ok(_) => drop(f),
        Err(_) => {
            let mut buf = String::new();

            if f.read_to_string(&mut buf).is_err() {
                buf = "<unreadable>".to_string();
            }

            return Err(StratisError::Msg(format!(
                "stratisd is already running with supposed pid: {buf}"
            )));
        }
    };

    Ok(stratisd_min_file)
}

fn main() -> Result<(), String> {
    fn main_box() -> Result<(), Box<dyn Error>> {
        let cmd = parse_args();
        let args = cmd.get_matches();

        let _stratisd_min_file = trylock_pid_file()?;

        let mut builder = Builder::new();
        if let Some(log_level) = args.get_one::<String>("log_level").map(|s| s.as_str()) {
            builder.filter(
                Some("stratisd"),
                LevelFilter::from_str(log_level)
                    .expect("argument parser only accepts valid log levels"),
            );
        } else if let Ok(s) = env::var("RUST_LOG") {
            builder.parse_filters(&s);
        }
        builder.init();

        run(args.get_flag("sim"))?;
        Ok(())
    }

    main_box().map_err(|e| e.to_string())
}

#[cfg(test)]
mod test {
    use super::parse_args;
    #[test]
    fn test_stratisd_min_parse_args() {
        parse_args().debug_assert();
    }
}
