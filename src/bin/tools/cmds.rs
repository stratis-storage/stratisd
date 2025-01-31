// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use clap::{ArgAction, Parser};

use crate::tools::{check_metadata, dump_metadata};

pub trait ToolCommand<'a> {
    fn name(&self) -> &'a str;
    fn run(&self, command_line_args: Vec<String>) -> Result<(), String>;
    fn show_in_after_help(&self) -> bool;
}

#[derive(Parser)]
#[command(
    version,
    name = "stratis-dumpmetadata",
    about = "Reads Stratis metadata from a Stratis device and displays it",
    next_line_help = true
)]
struct StratisDumpMetadataCli {
    /// Print metadata of given device
    #[arg(required = true)]
    dev: PathBuf,

    /// Print byte buffer of signature block
    #[arg(action = ArgAction::SetTrue, long="print-bytes", num_args=0, short='b')]
    print_bytes: bool,

    /// Only print specified portion of the metadata
    #[arg(action = ArgAction::Set, long="only", value_name = "PORTION", value_parser=["pool"])]
    only: Option<String>,
}

struct StratisDumpMetadata;

impl<'a> ToolCommand<'a> for StratisDumpMetadata {
    fn name(&self) -> &'a str {
        "stratis-dumpmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisDumpMetadataCli::parse_from(command_line_args);
        dump_metadata::run(
            &matches.dev,
            matches.print_bytes,
            matches.only.map(|v| v == "pool").unwrap_or(false),
        )
    }

    fn show_in_after_help(&self) -> bool {
        true
    }
}

#[derive(Parser)]
#[command(
    version,
    name = "stratis-checkmetadata",
    about = "Check validity of Stratis metadata",
    next_line_help = true
)]
struct StratisCheckMetadataCli {
    /// File containing pool-level metadata as JSON
    #[arg(required = true)]
    file: PathBuf,
}

struct StratisCheckMetadata;

impl<'a> ToolCommand<'a> for StratisCheckMetadata {
    fn name(&self) -> &'a str {
        "stratis-checkmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisCheckMetadataCli::parse_from(command_line_args);
        check_metadata::run(&matches.file, false)
    }

    fn show_in_after_help(&self) -> bool {
        false
    }
}

#[derive(Parser)]
#[command(
    version,
    name = "stratis-printmetadata",
    about = "Print a human-suitable representation of Stratis metadata",
    next_line_help = true
)]
struct StratisPrintMetadataCli {
    /// File containing pool-level metadata as JSON
    #[arg(required = true)]
    file: PathBuf,
}

struct StratisPrintMetadata;

impl<'a> ToolCommand<'a> for StratisPrintMetadata {
    fn name(&self) -> &'a str {
        "stratis-printmetadata"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), String> {
        let matches = StratisPrintMetadataCli::parse_from(command_line_args);
        check_metadata::run(&matches.file, true)
    }

    fn show_in_after_help(&self) -> bool {
        false
    }
}

pub fn cmds<'a>() -> Vec<Box<dyn ToolCommand<'a>>> {
    vec![
        Box::new(StratisCheckMetadata),
        Box::new(StratisDumpMetadata),
        Box::new(StratisPrintMetadata),
    ]
}

#[cfg(test)]
mod tests {

    use clap::CommandFactory;

    use super::{StratisCheckMetadataCli, StratisDumpMetadataCli, StratisPrintMetadataCli};

    #[test]
    fn test_dumpmetadata_parse_args() {
        StratisCheckMetadataCli::command().debug_assert();
        StratisDumpMetadataCli::command().debug_assert();
        StratisPrintMetadataCli::command().debug_assert();
    }
}
