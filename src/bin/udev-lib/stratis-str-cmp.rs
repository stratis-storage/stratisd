// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, Command};
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

/// Compare two strings and output on stdout 0 if they match and 1 if they do not.
fn string_compare(arg1: &str, arg2: &str) {
    if arg1 == arg2 {
        println!("0");
    } else {
        println!("1");
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();

    let parser = Command::new("stratis-str-cmp")
        .arg(
            Arg::new("left")
                .help("First string to compare")
                .required(true),
        )
        .arg(
            Arg::new("right")
                .help("Second string to compare")
                .required(true),
        );
    let matches = parser.get_matches_from(&args);
    string_compare(
        matches.value_of("left").expect("required argument"),
        matches.value_of("right").expect("required argument"),
    );

    Ok(())
}
