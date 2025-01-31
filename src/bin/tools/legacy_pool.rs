// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use clap::ArgMatches;
use serde_json::{json, Map, Value};

use stratisd::{
    engine::{
        register_clevis_token, EncryptionInfo, KeyDescription, ProcessedPathInfos, StratPool,
        CLEVIS_TANG_TRUST_URL,
    },
    stratis::StratisResult,
};

type ParseReturn = StratisResult<(
    String,
    Vec<PathBuf>,
    Option<KeyDescription>,
    Option<(String, Value)>,
)>;

fn parse_args(matches: &ArgMatches) -> ParseReturn {
    let pool_name = matches
        .get_one::<String>("pool_name")
        .expect("required")
        .clone();
    let blockdevs = matches
        .get_many::<PathBuf>("blockdevs")
        .expect("required")
        .cloned()
        .collect::<Vec<_>>();
    let key_desc = match matches.get_one::<String>("key_desc") {
        Some(kd) => Some(KeyDescription::try_from(kd)?),
        None => None,
    };
    let pin = matches.get_one::<String>("clevis");
    let clevis_info = match pin.map(|s| s.as_str()) {
        Some("nbde" | "tang") => {
            let mut json = Map::new();
            json.insert(
                "url".to_string(),
                Value::from(
                    matches
                        .get_one::<String>("tang_url")
                        .expect("Required")
                        .clone(),
                ),
            );
            if matches.get_flag("trust_url") {
                json.insert(CLEVIS_TANG_TRUST_URL.to_string(), Value::from(true));
            } else if let Some(thp) = matches.get_one::<String>("thumbprint") {
                json.insert("thp".to_string(), Value::from(thp.clone()));
            }
            pin.map(|p| (p.to_string(), Value::from(json)))
        }
        Some("tpm2") => Some(("tpm2".to_string(), json!({}))),
        Some(_) => unreachable!("Validated by parser"),
        None => None,
    };

    Ok((pool_name, blockdevs, key_desc, clevis_info))
}

pub fn run(matches: &ArgMatches) -> StratisResult<()> {
    let (name, devices, key_desc, clevis_info) = parse_args(matches)?;
    let unowned = ProcessedPathInfos::try_from(
        devices
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<_>>()
            .as_slice(),
    )?
    .unpack()
    .1;
    let encryption_info = match (key_desc, clevis_info) {
        (Some(kd), Some(ci)) => Some(EncryptionInfo::Both(kd, ci)),
        (Some(kd), _) => Some(EncryptionInfo::KeyDesc(kd)),
        (_, Some(ci)) => Some(EncryptionInfo::ClevisInfo(ci)),
        (_, _) => None,
    };
    register_clevis_token()?;
    StratPool::initialize(name.as_str(), unowned, encryption_info.as_ref())?;
    Ok(())
}
