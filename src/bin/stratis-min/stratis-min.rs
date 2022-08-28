// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, path::PathBuf};

use clap::{Arg, ArgGroup, ArgMatches, Command};
use serde_json::{json, Map, Value};

use stratisd::{
    engine::{EncryptionInfo, KeyDescription, PoolUuid, UnlockMethod, CLEVIS_TANG_TRUST_URL},
    jsonrpc::client::{filesystem, key, pool, report},
    stratis::{StratisError, VERSION},
};

fn parse_args() -> Command<'static> {
    Command::new("stratis-min")
        .version(VERSION)
        .arg(
            Arg::new("debug")
                .long("--debug")
                .takes_value(false)
                .required(false),
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommands(vec![
            Command::new("key").subcommands(vec![
                Command::new("set")
                    .group(
                        ArgGroup::new("key_method")
                            .arg("capture_key")
                            .arg("keyfile_path")
                            .required(true),
                    )
                    .arg(
                        Arg::new("capture_key")
                            .long("--capture-key")
                            .takes_value(false),
                    )
                    .arg(
                        Arg::new("keyfile_path")
                            .long("--keyfile-path")
                            .takes_value(true),
                    )
                    .arg(Arg::new("key_desc").required(true)),
                Command::new("list"),
                Command::new("unset").arg(Arg::new("key_desc").required(true)),
            ]),
            Command::new("pool").subcommands(vec![
                Command::new("start")
                    .arg(Arg::new("pool_uuid").required(true))
                    .arg(
                        Arg::new("unlock_method")
                            .long("--unlock-method")
                            .takes_value(true),
                    )
                    .arg(
                        Arg::new("prompt")
                            .long("--prompt")
                            .takes_value(false)
                            .requires("unlock_method"),
                    ),
                Command::new("stop").arg(Arg::new("pool_uuid").required(true)),
                Command::new("create")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .multiple_occurrences(true)
                            .required(true),
                    )
                    .arg(Arg::new("key_desc").long("--key-desc").takes_value(true))
                    .arg(
                        Arg::new("clevis")
                            .long("--clevis")
                            .takes_value(true)
                            .possible_values(&["nbde", "tang", "tpm2"])
                            .requires_if("nbde", "tang_args")
                            .requires_if("tang", "tang_args"),
                    )
                    .arg(
                        Arg::new("tang_url")
                            .long("--tang-url")
                            .takes_value(true)
                            .required_if_eq("clevis", "nbde")
                            .required_if_eq("clevis", "tang"),
                    )
                    .arg(
                        Arg::new("thumbprint")
                            .long("--thumbprint")
                            .takes_value(true),
                    )
                    .arg(Arg::new("trust_url").long("--trust-url").takes_value(false))
                    .group(
                        ArgGroup::new("tang_args")
                            .arg("thumbprint")
                            .arg("trust_url"),
                    ),
                Command::new("init-cache")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .multiple_occurrences(true)
                            .required(true),
                    ),
                Command::new("rename")
                    .arg(Arg::new("current_name").required(true))
                    .arg(Arg::new("new_name").required(true)),
                Command::new("add-data")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .multiple_occurrences(true)
                            .required(true),
                    ),
                Command::new("add-cache")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .multiple_occurrences(true)
                            .required(true),
                    ),
                Command::new("destroy").arg(Arg::new("name").required(true)),
                Command::new("is-encrypted").arg(Arg::new("pool_uuid").required(true)),
                Command::new("is-stopped").arg(Arg::new("pool_uuid").required(true)),
                Command::new("is-bound").arg(Arg::new("pool_uuid").required(true)),
                Command::new("has-passphrase").arg(Arg::new("pool_uuid").required(true)),
                Command::new("clevis-pin").arg(Arg::new("pool_uuid").required(true)),
            ]),
            Command::new("filesystem").subcommands(vec![
                Command::new("create")
                    .arg(Arg::new("pool_name").required(true))
                    .arg(Arg::new("fs_name").required(true)),
                Command::new("destroy")
                    .arg(Arg::new("pool_name").required(true))
                    .arg(Arg::new("fs_name").required(true)),
                Command::new("rename")
                    .arg(Arg::new("pool_name").required(true))
                    .arg(Arg::new("fs_name").required(true))
                    .arg(Arg::new("new_fs_name").required(true)),
            ]),
            Command::new("report"),
        ])
}

fn get_paths_from_args(args: &ArgMatches) -> Vec<PathBuf> {
    args.values_of("blockdevs")
        .expect("required")
        .map(PathBuf::from)
        .collect::<Vec<_>>()
}

fn main() -> Result<(), String> {
    fn main_box() -> Result<(), Box<dyn Error>> {
        let cmd = parse_args();

        let args = cmd.get_matches();
        if let Some(subcommand) = args.subcommand_matches("key") {
            if let Some(args) = subcommand.subcommand_matches("set") {
                key::key_set(
                    KeyDescription::try_from(
                        args.value_of("key_desc").expect("required").to_string(),
                    )?,
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
            if let Some(args) = subcommand.subcommand_matches("start") {
                let uuid = PoolUuid::parse_str(args.value_of("pool_uuid").expect("required"))?;
                let unlock_method = match args.value_of("unlock_method") {
                    Some(um) => Some(UnlockMethod::try_from(um)?),
                    None => None,
                };
                let prompt = args.is_present("prompt");
                if prompt && unlock_method == Some(UnlockMethod::Clevis) {
                    return Err(Box::new(StratisError::Msg(
                        "--prompt and an unlock_method of clevis are mutally exclusive".to_string(),
                    )));
                }
                pool::pool_start(uuid, unlock_method, prompt)?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("stop") {
                let uuid = PoolUuid::parse_str(args.value_of("pool_uuid").expect("required"))?;
                pool::pool_stop(uuid)?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("create") {
                let paths = get_paths_from_args(args);
                let key_description = match args.value_of("key_desc").map(|s| s.to_owned()) {
                    Some(string) => Some(KeyDescription::try_from(string)?),
                    None => None,
                };
                let pin = args.value_of("clevis");
                let clevis_info = match pin {
                    Some("nbde" | "tang") => {
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
                    EncryptionInfo::from_options((key_description, clevis_info)),
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
            } else if let Some(args) = subcommand.subcommand_matches("is-stopped") {
                let uuid_str = args.value_of("pool_uuid").expect("required");
                let uuid = PoolUuid::parse_str(uuid_str)?;
                println!("{}", pool::pool_is_stopped(uuid)?,);
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
            unreachable!("Parser requires a subcommand.")
        }
    }

    main_box().map_err(|e| e.to_string())
}
