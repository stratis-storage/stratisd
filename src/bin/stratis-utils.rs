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
