// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, error::Error, path::PathBuf};

use clap::{App, Arg, ArgGroup, ArgMatches, SubCommand};

use libstratis::{
    engine::{KeyDescription, PoolUuid},
    jsonrpc::client::{key, pool, report, udev},
};

fn parse_args() -> App<'static, 'static> {
    App::new("stratis-min")
        .arg(
            Arg::with_name("debug")
                .long("--debug")
                .takes_value(false)
                .required(false),
        )
        .subcommands(vec![
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
                    .arg(Arg::with_name("key_desc").required(true)),
                SubCommand::with_name("list"),
                SubCommand::with_name("unset").arg(Arg::with_name("key_desc").required(true)),
            ]),
            SubCommand::with_name("pool").subcommands(vec![
                SubCommand::with_name("unlock")
                    .arg(Arg::with_name("pool_uuid").required(true))
                    .arg(Arg::with_name("prompt").long("--prompt").takes_value(false)),
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
                SubCommand::with_name("is-encrypted")
                    .arg(Arg::with_name("pool_uuid").required(true)),
            ]),
            SubCommand::with_name("report"),
            SubCommand::with_name("udev").arg(Arg::with_name("dm_name").required(true)),
        ])
}

fn get_paths_from_args<'a>(args: &'a ArgMatches<'a>) -> Vec<PathBuf> {
    args.values_of("blockdevs")
        .expect("required")
        .map(PathBuf::from)
        .collect::<Vec<_>>()
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = parse_args();
    let mut help = Vec::new();
    app.write_long_help(&mut help)?;
    let help = String::from_utf8(help)?;
    let args = app.get_matches();
    if let Some(subcommand) = args.subcommand_matches("key") {
        if let Some(args) = subcommand.subcommand_matches("set") {
            key::key_set(
                KeyDescription::try_from(args.value_of("key_desc").expect("required").to_string())?,
                args.value_of("keyfile_path"),
            )?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("unset") {
            key::key_unset(KeyDescription::try_from(
                args.value_of("key_desc").expect("required").to_string(),
            )?)?;
            Ok(())
        } else {
            key::key_list()?;
            Ok(())
        }
    } else if let Some(subcommand) = args.subcommand_matches("pool") {
        if let Some(args) = subcommand.subcommand_matches("unlock") {
            let uuid = PoolUuid::parse_str(args.value_of("pool_uuid").expect("required"))?;
            pool::pool_unlock(uuid, args.is_present("prompt"))?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("create") {
            let paths = get_paths_from_args(args);
            pool::pool_create(
                args.value_of("name").expect("required").to_string(),
                paths,
                match args.value_of("key_desc").map(|s| s.to_owned()) {
                    Some(string) => Some(KeyDescription::try_from(string)?),
                    None => None,
                },
            )?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("destroy") {
            pool::pool_destroy(args.value_of("name").expect("required").to_string())?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("init-cache") {
            let paths = get_paths_from_args(args);
            pool::pool_init_cache(args.value_of("name").expect("required").to_string(), paths)?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("rename") {
            pool::pool_rename(
                args.value_of("current_name").expect("required").to_string(),
                args.value_of("new_name").expect("required").to_string(),
            )?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("add-data") {
            let paths = get_paths_from_args(args);
            pool::pool_add_data(args.value_of("name").expect("required").to_string(), paths)?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("add-cache") {
            let paths = get_paths_from_args(args);
            pool::pool_add_cache(args.value_of("name").expect("required").to_string(), paths)?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("is-encrypted") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str)?;
            println!("{}", pool::pool_is_encrypted(uuid)?,);
            Ok(())
        } else {
            pool::pool_list()?;
            Ok(())
        }
    } else if let Some("report") = args.subcommand_name() {
        report::report().and_then(|j| {
            println!("{}", serde_json::to_string_pretty(&j)?);
            Ok(())
        })?;
        Ok(())
    } else if let Some(args) = args.subcommand_matches("udev") {
        if let Some((pool_name, fs_name)) =
            udev::udev(args.value_of("dm_name").expect("required").to_string())?
        {
            println!("STRATIS_SYMLINK=stratis/{}/{}", pool_name, fs_name);
        }
        Ok(())
    } else {
        println!("{}", help);
        Ok(())
    }
}
