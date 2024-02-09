// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    error::Error,
    fmt::{self, Display},
    path::PathBuf,
};

use clap::{Arg, ArgAction, Command};

#[cfg(feature = "systemd_compat")]
use clap::builder::Str;

use devicemapper::Bytes;

use crate::utils::new_tool;
use crate::utils::predict_usage;

#[cfg(feature = "systemd_compat")]
use crate::utils::generators::{stratis_clevis_setup_generator, stratis_setup_generator};

#[derive(Debug)]
pub struct ExecutableError(pub String);

impl Display for ExecutableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ExecutableError {}

pub trait UtilCommand<'a> {
    fn name(&self) -> &'a str;
    fn run(&self, command_line_args: Vec<String>) -> Result<(), Box<dyn Error>>;
}

struct StratisPredictUsage;

impl StratisPredictUsage {
    fn cmd() -> Command {
        Command::new("stratis-predict-usage")
            .about("Predicts space usage for Stratis.")
            .subcommand_required(true)
            .subcommands(vec![
                Command::new("pool")
                    .about("Predicts the space usage when creating a Stratis pool.")
                    .arg(Arg::new("encrypted")
                        .long("encrypted")
                        .action(ArgAction::SetTrue)
                        .help("Whether the pool will be encrypted."),
                    )
                    .arg(
                        Arg::new("no-overprovision")
                        .long("no-overprovision")
                        .action(ArgAction::SetTrue)
                        .help("Indicates that the pool does not allow overprovisioning"),
                    )
                    .arg(
                        Arg::new("device-size")
                            .long("device-size")
                            .num_args(1)
                            .action(ArgAction::Append)
                            .required(true)
                            .help("Size of device to be included in the pool. May be specified multiple times. Units are bytes.")
                            .next_line_help(true)
                    )
                    .arg(
                        Arg::new("filesystem-size")
                        .long("filesystem-size")
                        .num_args(1)
                        .action(ArgAction::Append)
                        .help("Size of filesystem to be made for this pool. May be specified multiple times, one for each filesystem. Units are bytes. Must be at least 512 MiB and less than 4 PiB.")
                        .next_line_help(true)
                    ),
                Command::new("filesystem")
                    .about("Predicts the space usage when creating a Stratis filesystem.")
                    .arg(
                        Arg::new("filesystem-size")
                        .long("filesystem-size")
                        .num_args(1)
                        .action(ArgAction::Append)
                        .required(true)
                        .help("Size of filesystem to be made for this pool. May be specified multiple times, one for each filesystem. Units are bytes. Must be at least 512 MiB and less than 4 PiB.")
                        .next_line_help(true)
                    )
                    .arg(
                        Arg::new("no-overprovision")
                        .long("no-overprovision")
                        .action(ArgAction::SetTrue)
                        .help("Indicates that the pool does not allow overprovisioning"),
                    )]
            )
    }
}

impl<'a> UtilCommand<'a> for StratisPredictUsage {
    fn name(&self) -> &'a str {
        "stratis-predict-usage"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        let matches = StratisPredictUsage::cmd().get_matches_from(command_line_args);
        match matches.subcommand() {
            Some(("pool", sub_m)) => predict_usage::predict_pool_usage(
                sub_m.get_flag("encrypted"),
                !sub_m.get_flag("no-overprovision"),
                sub_m
                    .get_many::<String>("device-size")
                    .map(|szs| {
                        szs.map(|sz| sz.parse::<u128>().map(Bytes))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .expect("required argument")?,
                sub_m
                    .get_many::<String>("filesystem-size")
                    .map(|szs| {
                        szs.map(|sz| sz.parse::<u128>().map(Bytes))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?,
            ),
            Some(("filesystem", sub_m)) => predict_usage::predict_filesystem_usage(
                !sub_m.get_flag("no-overprovision"),
                sub_m
                    .get_many::<String>("filesystem-size")
                    .map(|szs| {
                        szs.map(|sz| sz.parse::<u128>().map(Bytes))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .expect("required argument")?,
            ),
            _ => unreachable!("Impossible subcommand name"),
        }
    }
}

#[cfg(feature = "systemd_compat")]
fn stratis_setup_generator_cmd(generator: impl Into<Str>) -> Command {
    Command::new(generator)
        .arg(
            Arg::new("normal_priority_dir")
                .required(true)
                .help("Directory in which to write a unit file for normal priority"),
        )
        .arg(
            Arg::new("early_priority_dir")
                .required(true)
                .help("Directory in which to write a unit file for early priority"),
        )
        .arg(
            Arg::new("late_priority_dir")
                .required(true)
                .help("Directory in which to write a unit file for late priority"),
        )
}

struct StratisSetupGenerator;

impl<'a> UtilCommand<'a> for StratisSetupGenerator {
    fn name(&self) -> &'a str {
        "stratis-setup-generator"
    }

    #[cfg(feature = "systemd_compat")]
    fn run(&self, command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        let matches = stratis_setup_generator_cmd("stratis-setup-generator")
            .get_matches_from(command_line_args);

        stratis_setup_generator::generator(
            matches
                .get_one::<String>("early_priority_dir")
                .expect("required")
                .to_owned(),
        )
    }

    #[cfg(not(feature = "systemd_compat"))]
    fn run(&self, _command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        Err(Box::new(ExecutableError(
            "systemd compatibility disabled for this build".into(),
        )))
    }
}

struct StratisClevisSetupGenerator;

impl<'a> UtilCommand<'a> for StratisClevisSetupGenerator {
    fn name(&self) -> &'a str {
        "stratis-clevis-setup-generator"
    }

    #[cfg(feature = "systemd_compat")]
    fn run(&self, command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        let matches = stratis_setup_generator_cmd("stratis-clevis-setup-generator")
            .get_matches_from(command_line_args);

        stratis_clevis_setup_generator::generator(
            matches
                .get_one::<String>("early_priority_dir")
                .expect("required")
                .to_owned(),
        )
    }

    #[cfg(not(feature = "systemd_compat"))]
    fn run(&self, _command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        Err(Box::new(ExecutableError(
            "systemd compatibility disabled for this build".into(),
        )))
    }
}

struct StratisNewTool;

impl StratisNewTool {
    fn cmd() -> Command {
        Command::new("stratis-new-tool")
            .about("Calculate Stratis-specific values from Stratis filesystem devicemapper path (format '/dev/mapper/<devicemapper-name>')")
                    .arg(Arg::new("path")
                        .required(true)
                        .value_name("PATH")
                        .help("The devicemapper path ('/dev/mapper/<devicemapper-name>')")
                    )
                    .arg(
                        Arg::new("output")
                        .required(true)
                        .long("output")
                        .value_name("OUTPUT_MODE")
                        .value_parser(["symlink", "pool", "filesystem"])
                        .help("Stratis value to print")
                    )
    }
}

impl<'a> UtilCommand<'a> for StratisNewTool {
    fn name(&self) -> &'a str {
        "stratis-new-tool"
    }

    fn run(&self, command_line_args: Vec<String>) -> Result<(), Box<dyn Error>> {
        let matches = StratisNewTool::cmd().get_matches_from(command_line_args);

        let path = matches
            .get_one::<String>("path")
            .expect("required argument")
            .parse::<PathBuf>()?;

        let output_mode: &String = matches.get_one("output").expect("required_argument");
        new_tool::print_value(&path, output_mode)?;

        Ok(())
    }
}

pub fn cmds<'a>() -> Vec<Box<dyn UtilCommand<'a>>> {
    vec![
        Box::new(StratisPredictUsage),
        Box::new(StratisSetupGenerator),
        Box::new(StratisClevisSetupGenerator),
        Box::new(StratisNewTool),
    ]
}

#[cfg(test)]
mod tests {

    use super::StratisPredictUsage;

    #[cfg(feature = "systemd_compat")]
    use super::stratis_setup_generator_cmd;

    #[test]
    fn test_predictusage_parse_args() {
        StratisPredictUsage::cmd().debug_assert();
    }

    #[test]
    #[cfg(feature = "systemd_compat")]
    fn test_generator_parse_args() {
        stratis_setup_generator_cmd("stratis-generator").debug_assert();
    }
}
