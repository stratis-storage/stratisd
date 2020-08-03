// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs::File, io, os::unix::io::AsRawFd};

use clap::{App, Arg, ArgGroup, SubCommand};

use libstratis::{
    engine::{DeleteAction, KeyActions, MappingCreateAction, StratKeyActions},
    stratis::{StratisError, StratisResult},
};

fn parse_args() -> App<'static, 'static> {
    App::new("stratis-min").subcommands(vec![SubCommand::with_name("key").subcommands(vec![
        SubCommand::with_name("set")
            .group(
                ArgGroup::with_name("key_method")
                    .arg("capture-key")
                    .arg("keyfile-path")
                    .required(true),
            )
            .arg(
                Arg::with_name("capture-key")
                    .long("--capture-key")
                    .takes_value(false),
            )
            .arg(
                Arg::with_name("keyfile-path")
                    .long("--keyfile-path")
                    .takes_value(true),
            )
            .arg(Arg::with_name("key_desc").required(true)),
        SubCommand::with_name("list"),
        SubCommand::with_name("unset").arg(Arg::with_name("key_desc").required(true)),
    ])])
}

fn key_set(key_desc: &str, keyfile_path: Option<&str>) -> StratisResult<()> {
    let ret = match keyfile_path {
        Some(kp) => {
            let file = File::open(kp)?;
            StratKeyActions.set(key_desc, file.as_raw_fd(), None)?
        }
        None => {
            let stdin_fd = io::stdin().as_raw_fd();
            println!("Enter desired key data followed by the return key:");
            StratKeyActions.set(key_desc, stdin_fd, Some(true))?
        }
    };
    match ret {
        MappingCreateAction::Identity => Err(StratisError::Error(format!(
            "Key with key description {} was unchanged by the set action",
            key_desc
        ))),
        MappingCreateAction::Created(()) => Ok(()),
        MappingCreateAction::ValueChanged(()) => Ok(()),
    }
}

fn key_unset(key_desc: &str) -> StratisResult<()> {
    match StratKeyActions.unset(key_desc)? {
        DeleteAction::Deleted(()) => Ok(()),
        DeleteAction::Identity => Err(StratisError::Error(format!(
            "Key with key description {} does not exist.",
            key_desc
        ))),
    }
}

fn key_list() -> StratisResult<()> {
    let keys = StratKeyActions.list()?;
    println!("Key description");
    for key in keys.iter() {
        println!("{}", key.as_application_str());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut app = parse_args();
    let mut help = Vec::new();
    app.write_long_help(&mut help).map_err(|e| e.to_string())?;
    let help = String::from_utf8(help).map_err(|e| e.to_string())?;
    let args = app.get_matches();
    if let Some(subcommand) = args.subcommand_matches("key") {
        if let Some(args) = subcommand.subcommand_matches("set") {
            key_set(
                args.value_of("key_desc").expect("required"),
                args.value_of("keyfile-path"),
            )
            .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("unset") {
            key_unset(args.value_of("key_desc").expect("required")).map_err(|e| e.to_string())
        } else {
            key_list().map_err(|e| e.to_string())
        }
    } else {
        println!("{}", help);
        Ok(())
    }
}
