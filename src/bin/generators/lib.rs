// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
};

pub fn get_generator_args() -> Result<(String, String, String), String> {
    let mut args = env::args();
    let normal_dir = args
        .nth(1)
        .ok_or_else(|| "Missing normal priority directory argument".to_string())?;
    let early_dir = args
        .next()
        .ok_or_else(|| "Missing early priority directory argument".to_string())?;
    let late_dir = args
        .next()
        .ok_or_else(|| "Missing late priority directory argument".to_string())?;
    Ok((normal_dir, early_dir, late_dir))
}

pub fn encode_path_to_device_unit(path: &Path) -> String {
    let mut encoded_path =
        path.display()
            .to_string()
            .chars()
            .skip(1)
            .fold(String::new(), |mut acc, c| {
                if c.is_alphanumeric() || c == '_' {
                    acc.push(c);
                } else if c == '/' {
                    acc.push('-');
                } else {
                    let buffer = &mut [0; 4];
                    let encoded_buffer = c.encode_utf8(buffer).as_bytes();
                    for byte in encoded_buffer.iter() {
                        acc += format!(r"\x{:x}", byte).as_str();
                    }
                }
                acc
            });
    encoded_path += ".device";
    encoded_path
}

pub fn write_unit_file(dest: &Path, file_contents: String) -> Result<(), io::Error> {
    let mut file = OpenOptions::new().write(true).create(true).open(dest)?;
    file.write_all(file_contents.as_bytes())?;
    Ok(())
}
