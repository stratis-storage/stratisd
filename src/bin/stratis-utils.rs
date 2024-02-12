// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod utils;

use std::{env, error::Error, path::Path};

use clap::{Arg, Command};

use crate::utils::{cmds, ExecutableError};

fn basename(path: &str) -> Option<&Path> {
    Path::new(path).file_name().map(Path::new)
}

fn main() -> Result<(), Box<dyn Error>> {
    let executable_name = "stratis-utils";

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
            return Err(Box::new(ExecutableError(
                "command name does not convert to string".to_string(),
            )));
        }
    };

    if let Some(c) = cmds().iter().find(|x| command_name == x.name()) {
        c.run(stripped_args)
    } else {
        Err(Box::new(ExecutableError(format!(
            "{command_name} is not a recognized executable name"
        ))))
    }
}
