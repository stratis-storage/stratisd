// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use clap::{Arg, ArgAction, Command};

use crate::tools::dump_metadata;

pub trait ToolCommand<'a> {
    fn name(&self) -> &'a str;
    fn run(&self, command_line_args: Vec<String>) -> Result<(), String>;
}

struct StratisDumpMetadata;

impl StratisDumpMetadata {
    fn cmd() -> Command {
        Command::new("stratis-dumpmetadata")
            .next_line_help(true)
            .arg(
                Arg::new("dev")
                    .value_parser(clap::value_parser!(PathBuf))
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
}

impl<'a> ToolCommand<'a> for StratisDumpMetadata {
    fn name(&self) -> &'a str {
        "stratis-dumpmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisDumpMetadata::cmd().get_matches_from(command_line_args);
        let devpath = matches
            .get_one::<PathBuf>("dev")
            .expect("'dev' is a mandatory argument");

        dump_metadata::run(
            devpath,
            matches.get_flag("print_bytes"),
            matches
                .get_one::<String>("only")
                .map(|v| v == "pool")
                .unwrap_or(false),
        )
    }
}

pub fn cmds<'a>() -> Vec<Box<dyn ToolCommand<'a>>> {
    vec![Box::new(StratisDumpMetadata)]
}

#[cfg(test)]
mod tests {

    use super::StratisDumpMetadata;

    #[test]
    fn test_dumpmetadata_parse_args() {
        StratisDumpMetadata::cmd().debug_assert();
    }
}
