// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, error::Error, path::PathBuf};

use clap::{Arg, ArgAction, ArgGroup, Command};
use serde_json::{json, Map, Value};

use stratisd::{
    engine::{
        InputEncryptionInfo, KeyDescription, Name, OptionalTokenSlotInput, PoolIdentifier,
        PoolUuid, TokenUnlockMethod, CLEVIS_TANG_TRUST_URL,
    },
    jsonrpc::client::{filesystem, key, pool, report},
    stratis::{StratisError, VERSION},
};

fn parse_args() -> Command {
    Command::new("stratis-min")
        .version(VERSION)
        .arg(Arg::new("debug").long("debug").num_args(0).required(false))
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
                    .arg(Arg::new("capture_key").long("capture-key").num_args(0))
                    .arg(Arg::new("keyfile_path").long("keyfile-path").num_args(1))
                    .arg(Arg::new("key_desc").required(true)),
                Command::new("list"),
                Command::new("unset").arg(Arg::new("key_desc").required(true)),
            ]),
            Command::new("pool").subcommands(vec![
                Command::new("start")
                    .arg(Arg::new("id").required(true))
                    .arg(Arg::new("name").long("name").num_args(0))
                    .arg(Arg::new("token_slot").long("token-slot").num_args(1))
                    .arg(Arg::new("prompt").long("prompt").num_args(0)),
                Command::new("stop")
                    .arg(Arg::new("id").required(true))
                    .arg(Arg::new("name").long("name").num_args(0)),
                Command::new("create")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .value_parser(clap::value_parser!(PathBuf))
                            .required(true),
                    )
                    .arg(
                        Arg::new("key_descs")
                            .long("key-descs")
                            .num_args(1)
                            .action(ArgAction::Append),
                    )
                    .arg(
                        Arg::new("clevis_infos")
                            .long("clevis-infos")
                            .num_args(1)
                            .action(ArgAction::Append),
                    ),
                Command::new("init-cache")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .value_parser(clap::value_parser!(PathBuf))
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
                            .value_parser(clap::value_parser!(PathBuf))
                            .required(true),
                    ),
                Command::new("add-cache")
                    .arg(Arg::new("name").required(true))
                    .arg(
                        Arg::new("blockdevs")
                            .action(ArgAction::Append)
                            .value_parser(clap::value_parser!(PathBuf))
                            .required(true),
                    ),
                Command::new("destroy").arg(Arg::new("name").required(true)),
                Command::new("is-encrypted")
                    .arg(Arg::new("name").long("name").num_args(0))
                    .arg(Arg::new("id").required(true)),
                Command::new("is-stopped")
                    .arg(Arg::new("name").long("name").num_args(0))
                    .arg(Arg::new("id").required(true)),
                Command::new("has-passphrase")
                    .arg(Arg::new("name").long("name").num_args(0))
                    .arg(Arg::new("id").required(true)),
                Command::new("bind")
                    .subcommand_required(true)
                    .subcommands(vec![
                        Command::new("keyring")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(
                                Arg::new("key_desc")
                                    .long("key-desc")
                                    .num_args(1)
                                    .required(true),
                            )
                            .arg(Arg::new("token_slot").long("token-slot").num_args(1)),
                        Command::new("nbde")
                            .alias("tang")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(Arg::new("tang_url").required(true))
                            .arg(Arg::new("token_slot").long("token-slot").num_args(1))
                            .arg(Arg::new("thumbprint").long("thumbprint").num_args(1))
                            .arg(Arg::new("trust_url").long("trust-url").num_args(0))
                            .group(
                                ArgGroup::new("tang_args")
                                    .arg("thumbprint")
                                    .arg("trust_url"),
                            ),
                        Command::new("tpm2")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(Arg::new("token_slot").long("token-slot").num_args(1)),
                    ]),
                Command::new("unbind")
                    .subcommand_required(true)
                    .subcommands(vec![
                        Command::new("keyring")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(
                                Arg::new("token_slot")
                                    .long("token-slot")
                                    .num_args(1)
                                    .value_parser(clap::value_parser!(u32)),
                            ),
                        Command::new("clevis")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(
                                Arg::new("token_slot")
                                    .long("token-slot")
                                    .num_args(1)
                                    .value_parser(clap::value_parser!(u32)),
                            ),
                    ]),
                Command::new("rebind")
                    .subcommand_required(true)
                    .subcommands(vec![
                        Command::new("keyring")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(
                                Arg::new("key_desc")
                                    .long("key-desc")
                                    .num_args(1)
                                    .required(true),
                            )
                            .arg(
                                Arg::new("token_slot")
                                    .long("token-slot")
                                    .num_args(1)
                                    .value_parser(clap::value_parser!(u32)),
                            ),
                        Command::new("clevis")
                            .arg(Arg::new("name").long("name").num_args(0))
                            .arg(Arg::new("id").required(true))
                            .arg(
                                Arg::new("token_slot")
                                    .long("token-slot")
                                    .num_args(1)
                                    .value_parser(clap::value_parser!(u32)),
                            ),
                    ]),
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
                Command::new("origin")
                    .arg(Arg::new("pool_name").required(true))
                    .arg(Arg::new("fs_name").required(true)),
            ]),
            Command::new("report"),
        ])
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
                let token_slot = match args.get_one::<String>("token_slot").map(|s| s.as_str()) {
                    Some("any") => TokenUnlockMethod::Any,
                    Some(s) => TokenUnlockMethod::Token(s.parse::<u32>()?),
                    None => TokenUnlockMethod::None,
                };
                let prompt = args.get_flag("prompt");
                pool::pool_start(id, token_slot, prompt)?;
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
                let kds = match args.get_many::<String>("key_descs") {
                    Some(key_descs) => {
                        key_descs.into_iter().try_fold(Vec::new(), |mut vec, s| {
                            let mut split = s.split(":");
                            let kd = split.next().ok_or_else(|| {
                                StratisError::Msg("Key description required".to_string())
                            })?;
                            let token_slot = split
                                .next()
                                .map(|i| {
                                    i.parse::<u32>()
                                        .map_err(|e| StratisError::Msg(e.to_string()))
                                })
                                .transpose()?;
                            vec.push((token_slot, KeyDescription::try_from(kd.to_string())?));
                            Result::<_, StratisError>::Ok(vec)
                        })?
                    }
                    None => vec![],
                };
                let cis = match args.get_many::<String>("clevis_infos") {
                    Some(clevis_infos) => {
                        clevis_infos
                            .into_iter()
                            .try_fold(Vec::new(), |mut vec, s| {
                                let pairs = s.split_whitespace().try_fold(
                                    HashMap::<&str, Option<&str>>::default(),
                                    |mut hm, pair| {
                                        let mut key_value = pair.split("=");
                                        match (key_value.next(), key_value.next()) {
                                            (None, _) => {
                                                return Err(StratisError::Msg(format!(
                                                    "Malformed value pair: {pair}"
                                                )));
                                            }
                                            (Some(k), None) => hm.insert(k, None),
                                            (Some(k), Some(v)) => hm.insert(k, Some(v)),
                                        };
                                        Ok(hm)
                                    },
                                )?;
                                let token_slot = if let Some(Some(t)) = pairs.get("token_slot") {
                                    Some(
                                        t.parse::<u32>()
                                            .map_err(|e| StratisError::Msg(e.to_string()))?,
                                    )
                                } else if let Some(None) = pairs.get("token_slot") {
                                    return Err(StratisError::Msg(
                                        "token_slot requires argument".to_string(),
                                    ));
                                } else {
                                    None
                                };
                                match pairs.get("pin") {
                                    Some(Some("tang")) => {
                                        let tang_url = if let Some(t) = pairs
                                            .get("tang_url")
                                            .and_then(|t| t.as_ref().map(|s| *s))
                                        {
                                            t
                                        } else {
                                            return Err(StratisError::Msg(
                                                "tang_url is required".to_string(),
                                            ));
                                        };
                                        let (thumbprint, trust_url) =
                                            match (pairs.get("thumbprint"), pairs.get("trust_url"))
                                            {
                                                (Some(Some(t)), None) => (Some(*t), false),
                                                (None, Some(None)) => (None, true),
                                                (None, Some(Some(_))) => {
                                                    return Err(StratisError::Msg(
                                                        "trust_url takes no argument".to_string(),
                                                    ));
                                                }
                                                (None, None) => {
                                                    return Err(StratisError::Msg(
                                                        "Missing required argument trust_url or thumbprint="
                                                            .to_string(),
                                                    ));
                                                }
                                                (_, _) => {
                                                    return Err(StratisError::Msg(
                                                        "thumbprint= cannot be used with trust_url"
                                                            .to_string(),
                                                    ));
                                                }
                                            };
                                        let mut json = Map::new();
                                        json.insert("url".to_string(), Value::from(tang_url));
                                        if trust_url {
                                            json.insert(
                                                CLEVIS_TANG_TRUST_URL.to_string(),
                                                Value::from(true),
                                            );
                                        } else if let Some(thp) = thumbprint {
                                            json.insert("thp".to_string(), Value::from(thp));
                                        }
                                        vec.push((
                                            token_slot,
                                            ("tang".to_string(), Value::from(json)),
                                        ));
                                        Ok(vec)
                                    }
                                    Some(Some("tpm2")) => {
                                        vec.push((token_slot, ("tpm2".to_string(), json!({}))));
                                        Ok(vec)
                                    }
                                    Some(Some(s)) => {
                                        Err(StratisError::Msg(format!("Invalid pin {s}")))
                                    }
                                    Some(None) => Err(StratisError::Msg(
                                        "Value required for key pin".to_string(),
                                    )),
                                    None => Err(StratisError::Msg("pin is required".to_string())),
                                }
                            })?
                    }
                    None => vec![],
                };
                let ei = InputEncryptionInfo::new(kds, cis)?;
                pool::pool_create(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    args.get_many::<PathBuf>("blockdevs")
                        .expect("required")
                        .cloned()
                        .collect::<Vec<_>>(),
                    ei,
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("destroy") {
                pool::pool_destroy(args.get_one::<String>("name").expect("required").to_owned())?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("init-cache") {
                pool::pool_init_cache(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    args.get_many::<PathBuf>("blockdevs")
                        .expect("required")
                        .cloned()
                        .collect::<Vec<_>>(),
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
                pool::pool_add_data(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    args.get_many::<PathBuf>("blockdevs")
                        .expect("required")
                        .cloned()
                        .collect::<Vec<_>>(),
                )?;
                Ok(())
            } else if let Some(args) = subcommand.subcommand_matches("add-cache") {
                pool::pool_add_cache(
                    args.get_one::<String>("name").expect("required").to_owned(),
                    args.get_many::<PathBuf>("blockdevs")
                        .expect("required")
                        .cloned()
                        .collect::<Vec<_>>(),
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
            } else if let Some(subcommand) = subcommand.subcommand_matches("bind") {
                if let Some(args) = subcommand.subcommand_matches("keyring") {
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
                    let key_desc = KeyDescription::try_from(
                        args.get_one::<String>("key_desc").expect("required"),
                    )?;
                    let token_slot = match args.get_one::<String>("token_slot").map(|s| s.as_str())
                    {
                        Some("legacy") => OptionalTokenSlotInput::Legacy,
                        Some(s) => OptionalTokenSlotInput::Some(s.parse::<u32>()?),
                        None => OptionalTokenSlotInput::None,
                    };
                    pool::pool_bind_keyring(id, token_slot, key_desc)?;
                    Ok(())
                } else if let Some(args) = subcommand.subcommand_matches("nbde") {
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
                    let clevis_info = {
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
                        Value::from(json)
                    };
                    let token_slot = match args.get_one::<String>("token_slot").map(|s| s.as_str())
                    {
                        Some("legacy") => OptionalTokenSlotInput::Legacy,
                        Some(s) => OptionalTokenSlotInput::Some(s.parse::<u32>()?),
                        None => OptionalTokenSlotInput::None,
                    };
                    pool::pool_bind_clevis(id, token_slot, "tang".to_string(), clevis_info)?;
                    Ok(())
                } else if let Some(args) = subcommand.subcommand_matches("tpm2") {
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
                    let pin = "tpm2";
                    let clevis_info = {
                        let json = Map::new();
                        Value::from(json)
                    };
                    let token_slot = match args.get_one::<String>("token_slot").map(|s| s.as_str())
                    {
                        Some("legacy") => OptionalTokenSlotInput::Legacy,
                        Some(s) => OptionalTokenSlotInput::Some(s.parse::<u32>()?),
                        None => OptionalTokenSlotInput::None,
                    };
                    pool::pool_bind_clevis(id, token_slot, pin.to_string(), clevis_info)?;
                    Ok(())
                } else {
                    unreachable!("Parser requires a subcommand")
                }
            } else if let Some(subcommand) = subcommand.subcommand_matches("unbind") {
                if let Some(args) = subcommand.subcommand_matches("keyring") {
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
                    let token_slot = args.get_one::<u32>("token_slot").cloned();
                    pool::pool_unbind_keyring(id, token_slot)?;
                    Ok(())
                } else if let Some(args) = subcommand.subcommand_matches("clevis") {
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
                    let token_slot = args.get_one::<u32>("token_slot").cloned();
                    pool::pool_unbind_clevis(id, token_slot)?;
                    Ok(())
                } else {
                    unreachable!("Parser requires a subcommand")
                }
            } else if let Some(subcommand) = subcommand.subcommand_matches("rebind") {
                if let Some(args) = subcommand.subcommand_matches("keyring") {
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
                    let key_desc = KeyDescription::try_from(
                        args.get_one::<String>("key_desc").expect("required"),
                    )?;
                    let token_slot = args.get_one::<u32>("token_slot").cloned();
                    pool::pool_rebind_keyring(id, token_slot, key_desc)?;
                    Ok(())
                } else if let Some(args) = subcommand.subcommand_matches("clevis") {
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
                    let token_slot = args.get_one::<u32>("token_slot").cloned();
                    pool::pool_rebind_clevis(id, token_slot)?;
                    Ok(())
                } else {
                    unreachable!("Parser requires a subcommand")
                }
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
            } else if let Some(args) = subcommand.subcommand_matches("origin") {
                filesystem::filesystem_origin(
                    args.get_one::<String>("pool_name")
                        .expect("required")
                        .to_owned(),
                    args.get_one::<String>("fs_name")
                        .expect("required")
                        .to_owned(),
                )
                .map(|origin| {
                    println!("{}", origin);
                })?;
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

#[cfg(test)]
mod test {
    use super::parse_args;
    #[test]
    fn test_stratis_min_parse_args() {
        parse_args().debug_assert();
    }
}
