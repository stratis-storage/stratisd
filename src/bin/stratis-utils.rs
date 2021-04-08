// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    error::Error,
    fmt::{self, Display},
    fs::OpenOptions,
    path::PathBuf,
};

use clap::{App, Arg};
use data_encoding::BASE32_NOPAD;
use serde_json::{json, Value};

use devicemapper::{Bytes, Sectors};

use libstratis::{
    engine::{blkdev_size, crypt_metadata_size, BDA},
    stratis::StratisResult,
};

#[derive(Debug)]
struct ExecutableError(String);

impl Display for ExecutableError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ExecutableError {}

/// Compare two strings and output on stdout 0 if they match and 1 if they do not.
fn string_compare(arg1: &str, arg2: &str) {
    if arg1 == arg2 {
        println!("0");
    } else {
        println!("1");
    }
}

fn base32_decode(var_name: &str, base32_str: &str) -> Result<(), Box<dyn Error>> {
    let base32_decoded = String::from_utf8(BASE32_NOPAD.decode(base32_str.as_bytes())?)?;
    println!("{}={}", var_name, base32_decoded);
    Ok(())
}

// Predict the free space that would be available in a pool given the
// following information:
// 1. Whether or not the pool is to be encrypted.
// 2. All the devices to be included in the pool.
fn predict_usage(encrypted: bool, devices: Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    let crypt_metadata_size = if encrypted {
        Bytes(u128::from(crypt_metadata_size()))
    } else {
        Bytes(0)
    };

    let crypt_metadata_size_sectors = crypt_metadata_size.sectors();

    // verify that crypt metadata size is divisible by sectors size
    assert_eq!(crypt_metadata_size_sectors.bytes(), crypt_metadata_size);

    let stratis_metadata_size_sectors = BDA::default().extended_size().sectors();

    let sizes = devices
        .iter()
        .map(|device| {
            OpenOptions::new()
                .read(true)
                .open(&device)
                .map_err(|e| e.into())
                .and_then(|f| blkdev_size(&f))
                // always truncated to sectors by stratisd
                .map(|s| s.sectors())
        })
        .collect::<StratisResult<Vec<_>>>()?;

    let total_size: Sectors = sizes.iter().cloned().sum();

    let stratis_device_sizes = sizes
        .iter()
        .map(|&s| s - crypt_metadata_size_sectors)
        .collect::<Vec<_>>();

    let stratis_avail_sizes = stratis_device_sizes
        .iter()
        .map(|&s| s - stratis_metadata_size_sectors)
        .collect::<Vec<_>>();

    let avail_size: Sectors = stratis_avail_sizes.iter().cloned().sum();

    let used_size = total_size - avail_size;

    let total_size_str = Value::String((*(total_size.bytes())).to_string());
    let used_size_str = Value::String((*(used_size.bytes())).to_string());
    let avail_size_str = Value::String((*(avail_size.bytes())).to_string());

    let json = json!(
        {"total": total_size_str, "used": used_size_str, "free": avail_size_str}
    );

    println!("{}", json);

    Ok(())
}

/// Parse the arguments based on which hard link was accessed.
fn parse_args() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    let argv1 = args[0].as_str();

    if argv1.ends_with("stratis-str-cmp") {
        if args.len() != 3 {
            return Err(Box::new(ExecutableError(format!(
                "{} requires two positional arguments",
                argv1
            ))));
        }

        string_compare(&args[1], &args[2]);
    } else if argv1.ends_with("stratis-base32-decode") {
        if args.len() != 3 {
            return Err(Box::new(ExecutableError(format!(
                "{} requires two positional arguments",
                argv1
            ))));
        }

        base32_decode(&args[1], &args[2])?;
    } else if argv1.ends_with("stratis-predict-usage") {
        let parser = App::new("stratis-predict-usage")
            .arg(
                Arg::with_name("encrypted")
                    .long("encrypted")
                    .help("Whether the pool will be encrypted"),
            )
            .arg(
                Arg::with_name("blockdevs")
                    .multiple(true)
                    .help("Devices to include in the pool"),
            );
        let matches = parser.get_matches_from(&args);
        predict_usage(
            matches.is_present("encrypted"),
            matches
                .values_of("blockdevs")
                .map(|bs| bs.map(PathBuf::from).collect::<Vec<_>>())
                .unwrap_or_default(),
        )?;
    } else {
        return Err(Box::new(ExecutableError(format!(
            "{} is not a recognized executable name",
            argv1
        ))));
    }

    Ok(())
}

/// This is the main method that dispatches the desired method based on the first
/// argument (the executable name). This will vary based on the hard link that was
/// invoked.
fn main() -> Result<(), Box<dyn Error>> {
    parse_args()
}
