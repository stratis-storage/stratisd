// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "systemd_compat")]
mod generators;

use std::{
    env,
    error::Error,
    fmt::{self, Display},
};

use clap::{App, Arg};
use data_encoding::BASE32_NOPAD;
use serde_json::{json, Value};

use devicemapper::{Bytes, Sectors};

use stratisd::engine::{crypt_metadata_size, ThinPoolSizeParams, BDA};

#[cfg(feature = "systemd_compat")]
use crate::generators::{stratis_clevis_setup_generator, stratis_setup_generator};

#[derive(Debug)]
struct ExecutableError(String);

impl Display for ExecutableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

// Predict usage for a newly created pool given information about whether
// or not the pool is encrypted, a list of device sizes, and an optional list
// of filesystem sizes.
fn predict_usage(
    encrypted: bool,
    device_sizes: Vec<Bytes>,
    filesystem_sizes: Option<Vec<Bytes>>,
) -> Result<(), Box<dyn Error>> {
    let crypt_metadata_size = if encrypted {
        Bytes(u128::from(crypt_metadata_size()))
    } else {
        Bytes(0)
    };

    let crypt_metadata_size_sectors = crypt_metadata_size.sectors();

    // verify that crypt metadata size is divisible by sector size
    assert_eq!(crypt_metadata_size_sectors.bytes(), crypt_metadata_size);

    let device_sizes = device_sizes.iter().map(|s| s.sectors()).collect::<Vec<_>>();

    let stratis_device_sizes = device_sizes
        .iter()
        .map(|&s| {
            (*s).checked_sub(*crypt_metadata_size_sectors)
                .map(Sectors)
                .ok_or_else(|| {
                    Box::new(ExecutableError(
                        "Some device sizes too small for encryption metadata.".into(),
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let stratis_metadata_size = BDA::default().extended_size().sectors();
    let stratis_avail_sizes = stratis_device_sizes
        .iter()
        .map(|&s| {
            (*s).checked_sub(*stratis_metadata_size)
                .map(Sectors)
                .ok_or_else(|| {
                    Box::new(ExecutableError(
                        "Some device sizes too small for Stratis metadata.".into(),
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let total_size: Sectors = device_sizes.iter().cloned().sum();
    let non_metadata_size: Sectors = stratis_avail_sizes.iter().cloned().sum();

    let size_params = ThinPoolSizeParams::new(non_metadata_size)?;
    let total_non_data = 2usize * size_params.meta_size() + size_params.mdv_size();

    let avail_size = (non_metadata_size)
        .checked_sub(*total_non_data)
        .map(Sectors)
        .ok_or_else(|| {
            Box::new(ExecutableError(
                "Sum of all device sizes too small for a Stratis pool.".into(),
            ))
        })?;

    // Guess at size required by specified filesystems. Due to layout effects
    // we can not predict precisely how much space will be required by xfs
    // metadata, etc. We use the following formulas which are based on some
    // observations of filesystem sizes on Stratis pools.
    // If logical size (ls) requested is:
    // <= 4 TiB, expected size use is 10 ^ (0.51 * log_10(ls) + 3)
    // otherwise, expected size is 10 ^ (0.36 * log_10(ls) + 5)
    // Sizes less than 512 MiB are rejected.
    // For our data, this computation always overestimates the amount of the
    // data required by the a filesystem, but always by less than 100%.
    // The function is piecewise, because our data had a pronounced kink at
    // 4 TiB. It is intuitively likely that, for larger
    // filesystems, the proportion of space used by the metadata should be
    // smaller, which justifies the piecewise nature somewhat.
    // This function is ad-hoc and can be re-evaluated if a better model
    // becomes available.

    fn space_for_filesystem(logical_size: Bytes) -> Sectors {
        let (m, b) = if logical_size < Bytes(4_398_046_511_104) {
            (0.51f64, 3.0f64)
        } else {
            (0.36f64, 5.0f64)
        };
        // Truncation is of no concern. All truncated values are expected to
        // have at least 10 digits to the left of the binary floating point
        // radix, the truncated fractional values will be relatively trivial.
        //
        // It is possible to lose precision when casting the logical size to
        // f64 when the logical size exceeds 4 PiB; this should not lead
        // to any computational misbehavior, as the log of the value should
        // change smoothly.
        #[allow(clippy::cast_possible_truncation)]
        #[allow(clippy::cast_precision_loss)]
        let result = Bytes(10f64.powf(m * f64::log10(*logical_size as f64) + b) as u128);

        let result_sectors = result.sectors();

        // Round up to the nearest sector
        if result_sectors.bytes() == result {
            result_sectors
        } else {
            result_sectors + Sectors(1)
        }
    }

    let fs_used: Option<Sectors> =
        filesystem_sizes.map(|sizes| sizes.iter().map(|&sz| space_for_filesystem(sz)).sum());

    let avail_size = (*avail_size)
        .checked_sub(*fs_used.unwrap_or(Sectors(0)))
        .map(Sectors)
        .ok_or_else(|| {
            Box::new(ExecutableError(
                "Filesystems will take up too much space on specified pool.".into(),
            ))
        })?;

    let used_size = total_size - avail_size;

    let total_size_str = Value::String((*(total_size.bytes())).to_string());
    let used_size_str = Value::String((*(used_size.bytes())).to_string());
    let avail_size_str = Value::String((*(avail_size.bytes())).to_string());

    let json = json! {
        {"total": total_size_str, "used": used_size_str, "free": avail_size_str}
    };

    println!("{}", json);

    Ok(())
}

/// Parse the arguments based on which hard link was accessed.
fn parse_args() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    let argv1 = args[0].as_str();

    if argv1.ends_with("stratis-str-cmp") {
        let parser = App::new("stratis-str-cmp")
            .arg(
                Arg::with_name("left")
                    .help("First string to compare")
                    .required(true),
            )
            .arg(
                Arg::with_name("right")
                    .help("Second string to compare")
                    .required(true),
            );
        let matches = parser.get_matches_from(&args);
        string_compare(
            matches.value_of("left").expect("required argument"),
            matches.value_of("right").expect("required argument"),
        );
    } else if argv1.ends_with("stratis-base32-decode") {
        let parser = App::new("stratis-base32-decode")
            .arg(
                Arg::with_name("key")
                    .help("Key for output string")
                    .required(true),
            )
            .arg(
                Arg::with_name("value")
                    .help("value to be decoded from base32 encoded sequence")
                    .required(true),
            );
        let matches = parser.get_matches_from(&args);
        base32_decode(
            matches.value_of("key").expect("required argument"),
            matches.value_of("value").expect("required argument"),
        )?;
    } else if argv1.ends_with("stratis-predict-usage") {
        let parser = App::new("stratis-predict-usage")
            .about("Predicts the space usage when creating a Stratis pool.")
            .arg(
                Arg::with_name("encrypted")
                    .long("encrypted")
                    .help("Whether the pool will be encrypted."),
            )
            .arg(
                Arg::with_name("device-size")
                    .long("device-size")
                    .number_of_values(1)
                    .multiple(true)
                    .required(true)
                    .help("Size of device to be included in the pool. May be specified multiple times. Units are bytes.")
                    .next_line_help(true)
            )
            .arg(
                Arg::with_name("filesystem-size")
                .long("filesystem-size")
                .multiple(true)
                .help("Size of filesystem to be made for this pool. May be specified multiple times, one for each filesystem. Units are bytes.")
                .next_line_help(true)
            );
        let matches = parser.get_matches_from(&args);
        predict_usage(
            matches.is_present("encrypted"),
            matches
                .values_of("device-size")
                .map(|szs| {
                    szs.map(|sz| sz.parse::<u128>().map(Bytes))
                        .collect::<Result<Vec<_>, _>>()
                })
                .expect("required argument")?,
            matches
                .values_of("filesystem-size")
                .map(|szs| {
                    szs.map(|sz| sz.parse::<u128>().map(Bytes))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?,
        )?;
    } else if argv1.ends_with("stratis-clevis-setup-generator")
        || argv1.ends_with("stratis-setup-generator")
    {
        #[cfg(feature = "systemd_compat")]
        if argv1.ends_with("stratis-clevis-setup-generator") {
            let parser = App::new("stratis-clevis-setup-generator")
                .arg(
                    Arg::with_name("normal_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for normal priority"),
                )
                .arg(
                    Arg::with_name("early_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for early priority"),
                )
                .arg(
                    Arg::with_name("late_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for late priority"),
                );
            let matches = parser.get_matches_from(&args);
            stratis_clevis_setup_generator::generator(
                matches
                    .value_of("early_priority_dir")
                    .expect("required")
                    .to_string(),
            )?;
        } else if argv1.ends_with("stratis-setup-generator") {
            let parser = App::new("stratis-setup-generator")
                .arg(
                    Arg::with_name("normal_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for normal priority"),
                )
                .arg(
                    Arg::with_name("early_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for early priority"),
                )
                .arg(
                    Arg::with_name("late_priority_dir")
                        .required(true)
                        .help("Directory in which to write a unit file for late priority"),
                );
            let matches = parser.get_matches_from(&args);
            stratis_setup_generator::generator(
                matches
                    .value_of("early_priority_dir")
                    .expect("required")
                    .to_string(),
            )?;
        }

        #[cfg(not(feature = "systemd_compat"))]
        return Err(Box::new(ExecutableError(
            "systemd compatibility disabled for this build".into(),
        )));
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
