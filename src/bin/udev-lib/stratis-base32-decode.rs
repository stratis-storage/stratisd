// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, Command};
use data_encoding::BASE32_NOPAD;
use std::{
    env,
    error::Error,
    fmt::{self, Display},
};

#[derive(Debug)]
struct ExecutableError(String);

impl Display for ExecutableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for ExecutableError {}

fn base32_decode(var_name: &str, base32_str: &str) -> Result<(), Box<dyn Error>> {
    let base32_decoded = String::from_utf8(BASE32_NOPAD.decode(base32_str.as_bytes())?)?;
    println!("{}={}", var_name, base32_decoded);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();

    let parser = Command::new("stratis-base32-decode")
        .arg(Arg::new("key").help("Key for output string").required(true))
        .arg(
            Arg::new("value")
                .help("value to be decoded from base32 encoded sequence")
                .required(true),
        );
    let matches = parser.get_matches_from(&args);
    base32_decode(
        matches.value_of("key").expect("required argument"),
        matches.value_of("value").expect("required argument"),
    )?;

    Ok(())
}
