// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    error::Error,
    fmt::{self, Display},
};

use data_encoding::BASE32_NOPAD;

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
