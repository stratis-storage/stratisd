// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use clap::{App, Arg, ArgGroup, ArgMatches, SubCommand};

use libstratis::engine::PoolUuid;

mod key;
mod pool;
mod report;
mod udev;
#[macro_use]
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
            SubCommand::with_name("setup").arg(
                Arg::with_name("pool_uuid")
                    .long("--pool-uuid")
                    .takes_value(true)
                    .required(false),
            ),
            SubCommand::with_name("create")
                .arg(Arg::with_name("name").required(true))
                .arg(Arg::with_name("blockdevs").multiple(true).required(true))
                .arg(
                    Arg::with_name("key_desc")
                        .long("--key-desc")
                        .takes_value(true),
                ),
            SubCommand::with_name("init-cache")
                .arg(Arg::with_name("name").required(true))
                .arg(Arg::with_name("blockdevs").multiple(true).required(true)),
            SubCommand::with_name("rename")
                .arg(Arg::with_name("current_name").required(true))
                .arg(Arg::with_name("new_name").required(true)),
            SubCommand::with_name("add-data")
                .arg(Arg::with_name("name").required(true))
                .arg(Arg::with_name("blockdevs").multiple(true).required(true)),
            SubCommand::with_name("destroy").arg(Arg::with_name("name").required(true)),
            SubCommand::with_name("is-encrypted").arg(Arg::with_name("pool_uuid").required(true)),
        ]),
        SubCommand::with_name("report"),
        SubCommand::with_name("udev").arg(Arg::with_name("dm_name").required(true)),
    ])
}

fn get_paths_from_args<'a>(args: &'a ArgMatches<'a>) -> Vec<&'a Path> {
    args.values_of("blockdevs")
        .expect("required")
        .map(|s| Path::new(s))
        .collect::<Vec<_>>()
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
        if let Some(args) = subcommand.subcommand_matches("setup") {
            let uuid_str = args.value_of("pool_uuid");
            let uuid = match uuid_str {
                Some(u) => Some(PoolUuid::parse_str(u).map_err(|e| e.to_string())?),
                None => None,
            };
            pool::pool_setup(uuid).map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("create") {
            let paths = get_paths_from_args(args);
            pool::pool_create(
                args.value_of("name").expect("required"),
                paths.as_slice(),
                args.value_of("key_desc").map(|s| s.to_owned()),
            )
            .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("destroy") {
            pool::pool_destroy(args.value_of("name").expect("required")).map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("init-cache") {
            let paths = get_paths_from_args(args);
            pool::pool_init_cache(args.value_of("name").expect("required"), paths.as_slice())
                .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("rename") {
            pool::pool_rename(
                args.value_of("current_name").expect("required"),
                args.value_of("new_name").expect("required"),
            )
            .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("add-data") {
            let paths = get_paths_from_args(args);
            pool::pool_add_data(args.value_of("name").expect("required"), paths.as_slice())
                .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("add-cache") {
            let paths = get_paths_from_args(args);
            pool::pool_add_cache(args.value_of("name").expect("required"), paths.as_slice())
                .map_err(|e| e.to_string())
        } else if let Some(args) = subcommand.subcommand_matches("is-encrypted") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str).map_err(|e| e.to_string())?;
            println!(
                "{}",
                pool::pool_is_encrypted(uuid).map_err(|e| e.to_string())?,
            );
            Ok(())
        } else {
            pool::pool_list().map_err(|e| e.to_string())
        }
    } else if let Some("report") = args.subcommand_name() {
        report::report().map_err(|e| e.to_string())
    } else if let Some(args) = args.subcommand_matches("udev") {
        udev::udev(args.value_of("dm_name").expect("required"))
    } else {
        println!("{}", help);
        Ok(())
    }
}
