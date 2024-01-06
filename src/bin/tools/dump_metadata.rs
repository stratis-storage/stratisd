// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom},
};

use pretty_hex::pretty_hex;

use serde_json::Value;

use stratisd::engine::{StaticHeader, StaticHeaderResult, BDA};

/// Format metadata on a given device
/// Returns StaticHeader fields
/// Returns an additional bytes buffer if print_bytes flag is True
fn fmt_metadata(shr: &StaticHeaderResult, print_bytes: bool) -> String {
    let mut result = String::from("\nHeader:\n")
        + shr
            .header
            .as_ref()
            .map_or(String::from("Unreadable\n"), |h| {
                h.as_ref().map_or_else(
                    |e| format!("Error: {}\n", e),
                    |s| {
                        s.as_ref()
                            .map_or(String::from("No signature buffer\n"), |sh| {
                                format!("{:#?}\n", sh)
                            })
                    },
                )
            })
            .as_str();
    if print_bytes {
        result += "\n\nBytes:\n\n";
        match &shr.bytes {
            Ok(ref boxed) => {
                result += pretty_hex(boxed.as_ref()).as_str();
            }
            Err(e) => {
                result += e.to_string().as_str();
            }
        }
    }

    result
}

/// Prints signature block
/// Prints the sigblock bytes if print_bytes is true.
/// Skips if only_pool is true.
fn print_signature_block(
    read_results: &(StaticHeaderResult, StaticHeaderResult),
    print_bytes: bool,
    only_pool: bool,
) {
    if only_pool {
        return;
    }

    if read_results.0 == read_results.1 {
        println!(
            "Signature block: \n{}",
            fmt_metadata(&read_results.0, print_bytes)
        );
    } else {
        println!(
            "Signature block 1: \n{}",
            fmt_metadata(&read_results.0, print_bytes)
        );
        println!(
            "Signature block 2: \n{}",
            fmt_metadata(&read_results.1, print_bytes)
        );
    };
}

/// Prints the bda.
/// Skips if only_pool is true.
fn print_bda(bda: &BDA, only_pool: bool) {
    if only_pool {
        return;
    }

    println!("\n{:#?}", bda);
}

/// Prints the pool level metadata.
/// Prints in machine-readable form if only_pool is true.
fn print_pool_metadata(pool_metadata: &Option<Vec<u8>>, only_pool: bool) -> Result<(), String> {
    if !only_pool {
        println!("\nPool metadata:");
    }
    if let Some(loaded_state) = pool_metadata {
        let state_json: Value = serde_json::from_slice(loaded_state)
            .map_err(|extract_err| format!("Error during state JSON extract: {}", extract_err))?;
        let state_json_pretty: String = serde_json::to_string_pretty(&state_json)
            .map_err(|parse_err| format!("Error during state JSON parse: {}", parse_err))?;
        println!("{}", state_json_pretty);
    } else if !only_pool {
        println!("None found");
    }

    Ok(())
}

// Print metadata, such as StaticHeaders, BDA, and Pool Metadata of given device.
// If sigblocks match, display the StaticHeader fields of a single sigblock,
// Otherwise display the StaticHeader fields of both sigblocks.
// If print_bytes flag is set to True, display the bytes buffer
// of the sigblock alongside the StaticHeader.
pub fn run(devpath: &str, print_bytes: bool, pool_only: bool) -> Result<(), String> {
    let mut devfile = OpenOptions::new()
        .read(true)
        .open(devpath)
        .map_err(|the_io_error| format!("Error opening device: {}", the_io_error))?;

    let read_results = StaticHeader::read_sigblocks(&mut devfile);
    print_signature_block(&read_results, print_bytes, pool_only);

    let header =
        StaticHeader::repair_sigblocks(&mut devfile, read_results, StaticHeader::do_nothing)
            .map_err(|repair_error| format!("No valid StaticHeader found: {}", repair_error))?
            .ok_or_else(|| "No valid Stratis signature found".to_string())?;

    let bda = BDA::load(header, &mut devfile)
        .map_err(|bda_load_error| format!("BDA detected but error found: {}", bda_load_error))?
        .ok_or_else(|| "No Stratis BDA metadata found".to_string())?;

    print_bda(&bda, pool_only);

    devfile
        .seek(SeekFrom::Start(0))
        .map_err(|seek_err| format!("Error during seek: {}", seek_err))?;

    let loaded_state = bda
        .load_state(&mut devfile)
        .map_err(|stateload_err| format!("Error during load state: {}", stateload_err))?;

    print_pool_metadata(&loaded_state, pool_only)?;

    Ok(())
}
