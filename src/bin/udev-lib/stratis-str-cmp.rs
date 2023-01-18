// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{env, error::Error};

/// Compare two strings and output on stdout 0 if they match and 1 if they do not.
fn string_compare(arg1: &str, arg2: &str) {
    if arg1 == arg2 {
        println!("0");
    } else {
        println!("1");
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let left = args
        .get(1)
        .ok_or("missing first argument, this program requires exactly 2 arguments")?;
    let right = args
        .get(2)
        .ok_or("missing second argument, this program requires exactly 2 arguments")?;

    string_compare(left, right);

    Ok(())
}
