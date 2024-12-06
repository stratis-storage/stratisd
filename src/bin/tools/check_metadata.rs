// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{fs, path::Path};

use stratisd::engine::pool_inspection::inspectors;

pub fn run(infile: &Path, print: bool) -> Result<(), String> {
    let metadata_str = fs::read_to_string(infile)
        .map_err(|the_io_error| format!("Error opening file: {}", the_io_error))?;
    let metadata = serde_json::from_str(&metadata_str)
        .map_err(|the_json_error| format!("Error parsing json into structs: {}", the_json_error))?;

    if print {
        inspectors::print(&metadata).map_err(|the_error| format!("Error: {}", the_error))
    } else {
        inspectors::check(&metadata).map_err(|the_error| format!("Error: {}", the_error))
    }
}
