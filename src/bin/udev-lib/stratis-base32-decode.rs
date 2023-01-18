// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use data_encoding::BASE32_NOPAD;
use std::{env, error::Error};

fn base32_decode(var_name: &str, base32_str: &str) -> Result<(), Box<dyn Error>> {
    let base32_decoded = String::from_utf8(BASE32_NOPAD.decode(base32_str.as_bytes())?)?;
    println!("{}={}", var_name, base32_decoded);
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let key = args
        .get(1)
        .ok_or("missing first argument, this program requires exactly 2 arguments")?;
    let value = args
        .get(2)
        .ok_or("missing second argument, this program requires exactly 2 arguments")?;

    base32_decode(key, value)?;

    Ok(())
}
