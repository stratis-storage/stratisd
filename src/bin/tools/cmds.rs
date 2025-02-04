// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgGroup, Command};

use crate::tools::{check_metadata, dump_metadata, legacy_pool};

use stratisd::stratis::VERSION;

pub trait ToolCommand<'a> {
    fn name(&self) -> &'a str;
    fn run(&self, command_line_args: Vec<String>) -> Result<(), String>;
    fn show_in_after_help(&self) -> bool;
}

struct StratisDumpMetadata;

impl StratisDumpMetadata {
    fn cmd() -> Command {
        Command::new("stratis-dumpmetadata")
            .version(VERSION)
            .about("Reads Stratis metadata from a Stratis device and displays it")
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

    fn show_in_after_help(&self) -> bool {
        true
    }
}

struct StratisCheckMetadata;

impl StratisCheckMetadata {
    fn cmd() -> Command {
        Command::new("stratis-checkmetadata")
            .version(VERSION)
            .about("Check validity of Stratis metadata")
            .next_line_help(true)
            .arg(
                Arg::new("file")
                    .value_parser(clap::value_parser!(PathBuf))
                    .required(true)
                    .help("File containing pool-level metadata as JSON"),
            )
    }
}

impl<'a> ToolCommand<'a> for StratisCheckMetadata {
    fn name(&self) -> &'a str {
        "stratis-checkmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisCheckMetadata::cmd().get_matches_from(command_line_args);
        let infile = matches
            .get_one::<PathBuf>("file")
            .expect("'file' is a mandatory argument");

        check_metadata::run(infile, false)
    }

    fn show_in_after_help(&self) -> bool {
        false
    }
}

struct StratisPrintMetadata;

impl StratisPrintMetadata {
    fn cmd() -> Command {
        Command::new("stratis-printmetadata")
            .version(VERSION)
            .about("Print a human-suitable representation of Stratis metadata")
            .next_line_help(true)
            .arg(
                Arg::new("file")
                    .value_parser(clap::value_parser!(PathBuf))
                    .required(true)
                    .help("File containing pool-level metadata as JSON"),
            )
    }
}

impl<'a> ToolCommand<'a> for StratisPrintMetadata {
    fn name(&self) -> &'a str {
        "stratis-printmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisPrintMetadata::cmd().get_matches_from(command_line_args);
        let infile = matches
            .get_one::<PathBuf>("file")
            .expect("'file' is a mandatory argument");

        check_metadata::run(infile, true)
    }

    fn show_in_after_help(&self) -> bool {
        false
    }
}

struct StratisLegacyPool;

impl StratisLegacyPool {
    fn cmd() -> Command {
        Command::new("stratis-legacy-pool")
            .version(VERSION)
            .about("A program for facilitating testing; not to be used in production. Creates a v1 pool equivalent to a pool that would be created by stratisd 3.7.3.")
            .arg(Arg::new("pool_name").num_args(1).required(true))
            .arg(
                Arg::new("blockdevs")
                    .action(ArgAction::Append)
                    .value_parser(clap::value_parser!(PathBuf))
                    .required(true),
            )
            .arg(
                Arg::new("key_desc")
                    .long("key-desc")
                    .num_args(1)
                    .required(false),
            )
            .arg(
                Arg::new("clevis")
                    .long("clevis")
                    .num_args(1)
                    .required(false)
                    .value_parser(["nbde", "tang", "tpm2"])
                    .requires_if("nbde", "tang_args")
                    .requires_if("tang", "tang_args"),
            )
            .arg(
                Arg::new("tang_url")
                    .long("tang-url")
                    .num_args(1)
                    .required_if_eq("clevis", "nbde")
                    .required_if_eq("clevis", "tang"),
            )
            .arg(Arg::new("thumbprint").long("thumbprint").num_args(1))
            .arg(Arg::new("trust_url").long("trust-url").num_args(0))
            .group(
                ArgGroup::new("tang_args")
                    .arg("thumbprint")
                    .arg("trust_url"),
            )
    }
}

impl<'a> ToolCommand<'a> for StratisLegacyPool {
    fn name(&self) -> &'a str {
        "stratis-legacy-pool"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisLegacyPool::cmd().get_matches_from(command_line_args);
        legacy_pool::run(&matches).map_err(|err| format!("{err}"))
    }

    fn show_in_after_help(&self) -> bool {
        false
    }
}

pub fn cmds<'a>() -> Vec<Box<dyn ToolCommand<'a>>> {
    vec![
        Box::new(StratisCheckMetadata),
        Box::new(StratisDumpMetadata),
        Box::new(StratisLegacyPool),
        Box::new(StratisPrintMetadata),
    ]
}

#[cfg(test)]
mod tests {

    use super::{StratisCheckMetadata, StratisDumpMetadata, StratisPrintMetadata};

    #[test]
    fn test_dumpmetadata_parse_args() {
        StratisCheckMetadata::cmd().debug_assert();
        StratisDumpMetadata::cmd().debug_assert();
        StratisPrintMetadata::cmd().debug_assert();
    }
}
