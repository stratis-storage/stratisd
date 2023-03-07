// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{error::Error, path::PathBuf};

use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};
use serde_json::{json, Map, Value};

use stratisd::{
    engine::{
        EncryptionInfo, KeyDescription, Name, PoolIdentifier, PoolUuid, UnlockMethod,
        CLEVIS_TANG_TRUST_URL,
    },
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
                    .arg(Arg::new("id").required(true))
                    .arg(Arg::new("name").long("--name").takes_value(false))
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
                Command::new("stop")
                    .arg(Arg::new("id").required(true))
                    .arg(Arg::new("name").long("--name").takes_value(false)),
                Command::new("create")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .required(true),
                    )
                    .arg(Arg::new("key_desc").long("--key-desc").takes_value(true))
                    .arg(
                        Arg::new("clevis")
                            .long("--clevis")
                            .takes_value(true)
                            .value_parser(["nbde", "tang", "tpm2"])
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
                            .action(ArgAction::Append)
                            .required(true),
                    ),
                Command::new("rename")
                    .arg(Arg::new("current_name").required(true))
                    .arg(Arg::new("new_name").required(true)),
                Command::new("add-data")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .required(true),
                    ),
                Command::new("add-cache")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .required(true),
                    ),
                Command::new("destroy").arg(Arg::new("name").required(true)),
                Command::new("is-encrypted")
                    .arg(Arg::new("name").long("--name").takes_value(false))
                    .arg(Arg::new("id").required(true)),
                Command::new("is-stopped")
                    .arg(Arg::new("name").long("--name").takes_value(false))
                    .arg(Arg::new("id").required(true)),
                Command::new("is-bound")
                    .arg(Arg::new("name").long("--name").takes_value(false))
                    .arg(Arg::new("id").required(true)),
                Command::new("has-passphrase")
                    .arg(Arg::new("name").long("--name").takes_value(false))
                    .arg(Arg::new("id").required(true)),
                Command::new("clevis-pin")
                    .arg(Arg::new("name").long("--name").takes_value(false))
                    .arg(Arg::new("id").required(true)),
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
    args.get_many::<String>("blockdevs")
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
                        args.get_one::<String>("key_desc").expect("required"),
                    )?,
                    args.get_one::<String>("keyfile_path").map(|s| s.as_str()),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("unset") {
                key::key_unset(KeyDescription::try_from(
                    args.get_one::<String>("key_desc").expect("required"),
                )?)?;
                Ok(())
            } else {
                key::key_list()?;
                Ok(())
            }
        } else if let Some(subcommand) = args.subcommand_matches("pool") {
            if let Some(args) = subcommand.subcommand_matches("start") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                let unlock_method =
                    match args.get_one::<String>("unlock_method").map(|s| s.as_str()) {
                        Some(um) => Some(UnlockMethod::try_from(um)?),
                        None => None,
                    };
                let prompt = args.get_flag("prompt");
                if prompt && unlock_method == Some(UnlockMethod::Clevis) {
                    return Err(Box::new(StratisError::Msg(
                        "--prompt and an unlock_method of clevis are mutally exclusive".to_string(),
                    )));
                }
                pool::pool_start(id, unlock_method, prompt)?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("stop") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                pool::pool_stop(id)?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("create") {
                let paths = get_paths_from_args(args);
                let key_description = match args.get_one::<String>("key_desc").map(|s| s.to_owned())
                {
                    Some(string) => Some(KeyDescription::try_from(string)?),
                    None => None,
                };
                let pin = args.get_one::<String>("clevis").map(|s| s.as_str());
                let clevis_info = match pin {
                    Some("nbde" | "tang") => {
                        let mut json = Map::new();
                        json.insert(
                            "url".to_string(),
                            Value::from(
                                args.get_one::<String>("tang_url")
                                    .map(|s| s.as_str())
                                    .expect("Required"),
                            ),
                        );
                        if args.get_flag("trust_url") {
                            json.insert(CLEVIS_TANG_TRUST_URL.to_string(), Value::from(true));
                        } else if let Some(thp) =
                            args.get_one::<String>("thumbprint").map(|s| s.as_str())
                        {
                            json.insert("thp".to_string(), Value::from(thp));
                        }
                        pin.map(|p| (p.to_string(), Value::from(json)))
                    }
                    Some("tpm2") => Some(("tpm2".to_string(), json!({}))),
                    Some(_) => unreachable!("Validated by parser"),
                    None => None,
                };
                pool::pool_create(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    paths,
                    EncryptionInfo::from_options((key_description, clevis_info)),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("destroy") {
                pool::pool_destroy(args.get_one::<String>("name").expect("required").to_owned())?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("init-cache") {
                let paths = get_paths_from_args(args);
                pool::pool_init_cache(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    paths,
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("rename") {
                pool::pool_rename(
                    args.get_one::<String>("current_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("new_name")
                        .expect("required")
                        .to_owned(),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("add-data") {
                let paths = get_paths_from_args(args);
                pool::pool_add_data(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    paths,
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("add-cache") {
                let paths = get_paths_from_args(args);
                pool::pool_add_cache(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    paths,
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("is-encrypted") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                println!("{}", pool::pool_is_encrypted(id)?);
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("is-stopped") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                println!("{}", pool::pool_is_stopped(id)?);
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("is-bound") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                println!("{}", pool::pool_is_bound(id)?);
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("has-passphrase") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                println!("{}", pool::pool_has_passphrase(id)?);
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("clevis-pin") {
                let id = if args.get_flag("name") {
                    PoolIdentifier::Name(Name::new(
                        args.get_one::<String>("id").expect("required").to_owned(),
                    ))
                } else {
                    PoolIdentifier::Uuid(PoolUuid::parse_str(
                        args.get_one::<String>("id")
                            .map(|s| s.as_str())
                            .expect("required"),
                    )?)
                };
                println!("{}", pool::pool_clevis_pin(id)?);
                Ok(())
            } else {
                pool::pool_list()?;
                Ok(())
            }
        } else if let Some(subcommand) = args.subcommand_matches("filesystem") {
            if let Some(args) = subcommand.subcommand_matches("create") {
                filesystem::filesystem_create(
                    args.get_one::<String>("pool_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("fs_name")
                        .expect("required")
                        .to_owned(),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("destroy") {
                filesystem::filesystem_destroy(
                    args.get_one::<String>("pool_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("fs_name")
                        .expect("required")
                        .to_owned(),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("rename") {
                filesystem::filesystem_rename(
                    args.get_one::<String>("pool_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("fs_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("new_fs_name")
                        .expect("required")
                        .to_owned(),
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
