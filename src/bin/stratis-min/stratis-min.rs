// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{convert::TryFrom, error::Error, path::PathBuf};

use clap::{App, Arg, ArgGroup, ArgMatches, SubCommand};
use serde_json::{json, Map, Value};

use libstratis::{
    engine::{EncryptionInfo, KeyDescription, PoolUuid, UnlockMethod, CLEVIS_TANG_TRUST_URL},
    jsonrpc::client::{filesystem, key, pool, report},
    stratis::{StratisError, VERSION},
};

fn parse_args() -> App<'static, 'static> {
    App::new("stratis-min")
        .version(VERSION)
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
                    .arg(Arg::with_name("unlock_method").required(true))
                    .arg(Arg::with_name("pool_uuid").required(false))
                    .arg(
                        Arg::with_name("prompt")
                            .long("--prompt")
                            .takes_value(false)
                            .requires("pool_uuid"),
                    ),
                SubCommand::with_name("create")
                    .arg(Arg::with_name("name").required(true))
                    .arg(Arg::with_name("blockdevs").multiple(true).required(true))
                    .arg(
                        Arg::with_name("key_desc")
                            .long("--key-desc")
                            .takes_value(true),
                    )
                    .arg(
                        Arg::with_name("clevis")
                            .long("--clevis")
                            .takes_value(true)
                            .possible_values(&["nbde", "tang", "tpm2"])
                            .requires_if("nbde", "tang_args")
                            .requires_if("tang", "tang_args"),
                    )
                    .arg(
                        Arg::with_name("tang_url")
                            .long("--tang-url")
                            .takes_value(true)
                            .required_if("clevis", "nbde")
                            .required_if("clevis", "tang"),
                    )
                    .arg(
                        Arg::with_name("thumbprint")
                            .long("--thumbprint")
                            .takes_value(true),
                    )
                    .arg(
                        Arg::with_name("trust_url")
                            .long("--trust-url")
                            .takes_value(false),
                    )
                    .group(
                        ArgGroup::with_name("tang_args")
                            .arg("thumbprint")
                            .arg("trust_url"),
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
                SubCommand::with_name("is-locked").arg(Arg::with_name("pool_uuid").required(true)),
                SubCommand::with_name("is-bound").arg(Arg::with_name("pool_uuid").required(true)),
                SubCommand::with_name("has-passphrase")
                    .arg(Arg::with_name("pool_uuid").required(true)),
                SubCommand::with_name("clevis-pin").arg(Arg::with_name("pool_uuid").required(true)),
            ]),
            SubCommand::with_name("filesystem").subcommands(vec![
                SubCommand::with_name("create")
                    .arg(Arg::with_name("pool_name").required(true))
                    .arg(Arg::with_name("fs_name").required(true)),
                SubCommand::with_name("destroy")
                    .arg(Arg::with_name("pool_name").required(true))
                    .arg(Arg::with_name("fs_name").required(true)),
                SubCommand::with_name("rename")
                    .arg(Arg::with_name("pool_name").required(true))
                    .arg(Arg::with_name("fs_name").required(true))
                    .arg(Arg::with_name("new_fs_name").required(true)),
            ]),
            SubCommand::with_name("report"),
        ])
}

fn get_paths_from_args<'a>(args: &'a ArgMatches<'a>) -> Vec<PathBuf> {
    args.values_of("blockdevs")
        .expect("required")
        .map(PathBuf::from)
        .collect::<Vec<_>>()
}

fn get_long_help(app: &mut App) -> Result<String, Box<dyn Error>> {
    let mut help = Vec::new();
    app.write_long_help(&mut help)?;
    Ok(String::from_utf8(help)?)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = parse_args();
    let long_help = get_long_help(&mut app)?;

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
            let unlock_method =
                UnlockMethod::try_from(args.value_of("unlock_method").expect("required"))?;
            let uuid = match args.value_of("pool_uuid") {
                Some(u) => Some(PoolUuid::parse_str(u)?),
                None => None,
            };
            let prompt = args.is_present("prompt");
            if prompt && unlock_method == UnlockMethod::Clevis {
                return Err(Box::new(StratisError::Msg(
                    "--prompt and an unlock_method of clevis are mutally exclusive".to_string(),
                )));
            }
            pool::pool_unlock(unlock_method, uuid, prompt)?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("create") {
            let paths = get_paths_from_args(args);
            let key_description = match args.value_of("key_desc").map(|s| s.to_owned()) {
                Some(string) => Some(KeyDescription::try_from(string)?),
                None => None,
            };
            let pin = args.value_of("clevis");
            let clevis_info = match pin {
                Some("nbde") | Some("tang") => {
                    let mut json = Map::new();
                    json.insert(
                        "url".to_string(),
                        Value::from(args.value_of("tang_url").expect("Required")),
                    );
                    if args.is_present("trust_url") {
                        json.insert(CLEVIS_TANG_TRUST_URL.to_string(), Value::from(true));
                    } else if let Some(thp) = args.value_of("thumbprint") {
                        json.insert("thp".to_string(), Value::from(thp));
                    }
                    pin.map(|p| (p.to_string(), Value::from(json)))
                }
                Some("tpm2") => Some(("tpm2".to_string(), json!({}))),
                Some(_) => unreachable!("Validated by parser"),
                None => None,
            };
            pool::pool_create(
                args.value_of("name").expect("required").to_string(),
                paths,
                EncryptionInfo {
                    key_description,
                    clevis_info,
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
        } else if let Some(args) = subcommand.subcommand_matches("is-locked") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str)?;
            println!("{}", pool::pool_is_locked(uuid)?,);
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("is-bound") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str)?;
            println!("{}", pool::pool_is_bound(uuid)?,);
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("has-passphrase") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str)?;
            println!("{}", pool::pool_has_passphrase(uuid)?,);
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("clevis-pin") {
            let uuid_str = args.value_of("pool_uuid").expect("required");
            let uuid = PoolUuid::parse_str(uuid_str)?;
            println!("{}", pool::pool_clevis_pin(uuid)?,);
            Ok(())
        } else {
            pool::pool_list()?;
            Ok(())
        }
    } else if let Some(subcommand) = args.subcommand_matches("filesystem") {
        if let Some(args) = subcommand.subcommand_matches("create") {
            filesystem::filesystem_create(
                args.value_of("pool_name").expect("required").to_string(),
                args.value_of("fs_name").expect("required").to_string(),
            )?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("destroy") {
            filesystem::filesystem_destroy(
                args.value_of("pool_name").expect("required").to_string(),
                args.value_of("fs_name").expect("required").to_string(),
            )?;
            Ok(())
        } else if let Some(args) = subcommand.subcommand_matches("rename") {
            filesystem::filesystem_rename(
                args.value_of("pool_name").expect("required").to_string(),
                args.value_of("fs_name").expect("required").to_string(),
                args.value_of("new_fs_name").expect("required").to_string(),
            )?;
            Ok(())
        } else {
            filesystem::filesystem_list()?;
            Ok(())
        }
    } else if let Some("report") = args.subcommand_name() {
        report::report().and_then(|j| {
            println!("{}", serde_json::to_string_pretty(&j)?);
            Ok(())
        })?;
        Ok(())
    } else {
        println!("{}", long_help);
        Ok(())
    }
}
