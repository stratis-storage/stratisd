// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod tools;

use std::{env, path::Path, process};

use clap::{Arg, Command};
use env_logger::Builder;

use crate::tools::cmds;

fn basename(path: &str) -> Option<&Path> {
    Path::new(path).file_name().map(Path::new)
}

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

fn main() {
    initialize_log();

    let executable_name = "stratisd-tools";

    let args = env::args().collect::<Vec<_>>();
    let argv1 = &args[0];

    let stripped_args = if basename(argv1.as_str())
        .map(|n| n == Path::new(executable_name))
        .unwrap_or(false)
    {
        let command = Command::new(executable_name)
            .arg(
                Arg::new("executable")
                    .required(true)
                    .value_name("EXECUTABLE")
                    .value_parser(cmds().iter().map(|x| x.name()).collect::<Vec<_>>()),
            )
            .arg_required_else_help(true);

        let truncated_args = if args.len() > 1 {
            vec![argv1, &args[1]]
        } else {
            vec![argv1]
        };

        command.get_matches_from(truncated_args);
        args[1..].to_vec()
    } else {
        args
    };

    let command_name = match basename(&stripped_args[0]).and_then(|n| n.to_str()) {
        Some(name) => name,
        None => {
            process::exit(1);
        }
    };

    if let Some(c) = cmds().iter().find(|x| command_name == x.name()) {
        match c.run(stripped_args) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error encountered: {e}");
                process::exit(1);
            }
        }
    } else {
        eprintln!("Unknown executable name {command_name}");
        process::exit(2);
    }
}
