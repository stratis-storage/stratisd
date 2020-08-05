// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use clap::{App, Arg, ArgGroup, SubCommand};

mod key;
mod pool;
mod utils;

fn parse_args() -> App<'static, 'static> {
    App::new("stratis-min").subcommands(vec![
        SubCommand::with_name("key").subcommands(vec![
            SubCommand::with_name("set")
                .group(
                    ArgGroup::with_name("key_method")
                        .arg("capture_key")
                        .arg("keyfile_path")
                        .required(true),
                )
                .arg(
                    Arg::with_name("capture_key")
                        .long("--capture-key")
                        .takes_value(false),
                )
                .arg(
                    Arg::with_name("keyfile_path")
                        .long("--keyfile-path")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("no_tty")
                        .long("--no-tty")
                        .takes_value(false)
                        .conflicts_with("keyfile_path"),
                )
                .arg(Arg::with_name("key_desc").required(true)),
            SubCommand::with_name("list"),
            SubCommand::with_name("unset").arg(Arg::with_name("key_desc").required(true)),
        ]),
        SubCommand::with_name("pool").subcommands(vec![
            SubCommand::with_name("setup"),
            SubCommand::with_name("create")
                .arg(Arg::with_name("name").required(true))
                .arg(Arg::with_name("blockdevs").multiple(true).required(true))
                .arg(
                    Arg::with_name("key_desc")
                        .long("--key-desc")
                        .takes_value(true),
                ),
        ]),
    ])
}

fn main() -> Result<(), String> {
    let mut app = parse_args();
    let mut help = Vec::new();
    app.write_long_help(&mut help).map_err(|e| e.to_string())?;
    let help = String::from_utf8(help).map_err(|e| e.to_string())?;
    let args = app.get_matches();
    if let Some(subcommand) = args.subcommand_matches("key") {
        if let Some(args) = subcommand.subcommand_matches("set") {
            key::key_set(
                args.value_of("key_desc").expect("required"),
                args.value_of("keyfile_path"),
                args.is_present("no_tty"),
            )
            .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("unset") {
            key::key_unset(args.value_of("key_desc").expect("required")).map_err(|e| e.to_string())
        } else {
            key::key_list().map_err(|e| e.to_string())
        }
    } else if let Some(subcommand) = args.subcommand_matches("pool") {
        if let Some("setup") = subcommand.subcommand_name() {
            pool::pool_setup().map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("create") {
            let paths = args
                .values_of("blockdevs")
                .expect("required")
                .map(|s| Path::new(s))
                .collect::<Vec<_>>();
            pool::pool_create(
                args.value_of("name").expect("required"),
                paths.as_slice(),
                args.value_of("key_desc").map(|s| s.to_owned()),
            )
            .map_err(|e| e.to_string())
        } else {
            pool::pool_list().map_err(|e| e.to_string())
        }
    } else {
        println!("{}", help);
        Ok(())
    }
}
