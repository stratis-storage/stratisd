// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod tools;

use std::{env, process};

use env_logger::Builder;

use clap::{Arg, ArgAction, Command};

use tools::dump_metadata;

/// Configure and initialize the logger.
/// Read log configuration parameters from the environment if RUST_LOG
/// is set. Otherwise, just accept the default configuration, which is
/// to log at the severity of error only.
fn initialize_log() {
    let mut builder = Builder::new();

    if let Ok(s) = env::var("RUST_LOG") {
        builder.parse_filters(&s);
    }

    builder.init()
}

fn parse_args() -> Command {
    Command::new("stratis-dumpmetadata")
        .next_line_help(true)
        .arg(
            Arg::new("dev")
                .required(true)
                .help("Print metadata of given device"),
        )
        .arg(
            Arg::new("print_bytes")
                .long("print-bytes")
                .action(ArgAction::SetTrue)
                .num_args(0)
                .short('b')
                .help("Print byte buffer of signature block"),
        )
        .arg(
            Arg::new("only")
                .long("only")
                .action(ArgAction::Set)
                .value_name("PORTION")
                .value_parser(["pool"])
                .help("Only print specified portion of the metadata"),
        )
}

fn main() {
    let matches = parse_args().get_matches();
    let devpath = matches
        .get_one::<String>("dev")
        .map(|s| s.as_str())
        .expect("'dev' is a mandatory argument");

    initialize_log();

    match dump_metadata::run(
        devpath,
        matches.get_flag("print_bytes"),
        matches
            .get_one::<String>("only")
            .map(|v| v == "pool")
            .unwrap_or(false),
    ) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn test_dumpmetadata_parse_args() {
        parse_args().debug_assert();
    }
}
