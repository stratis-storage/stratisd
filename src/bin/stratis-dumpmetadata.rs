// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    process,
};

use env_logger::Builder;

use clap::{Arg, Command};

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

/// Configure and initialize the logger.
/// Read log configuration parameters from the environment if RUST_LOG
/// is set. Otherwise, just accept the default configuration, which is
/// to log at the severity of error only.
fn initialize_log() {
    let mut builder = Builder::new();

    if let Ok(s) = env::var("RUST_LOG") {
        builder.parse_filters(&s);
    }

    builder.init()
}

// Print metadata, such as StaticHeaders, BDA, and Pool Metadata of given device.
// If sigblocks match, display the StaticHeader fields of a single sigblock,
// Otherwise display the StaticHeader fields of both sigblocks.
// If print_bytes flag is set to True, display the bytes buffer
// of the sigblock alongside the StaticHeader.
fn run(devpath: &str, print_bytes: bool) -> Result<(), String> {
    let mut devfile = OpenOptions::new()
        .read(true)
        .open(&devpath)
        .map_err(|the_io_error| format!("Error opening device: {}", the_io_error))?;

    let read_results = StaticHeader::read_sigblocks(&mut devfile);
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
    }

    let header =
        StaticHeader::repair_sigblocks(&mut devfile, read_results, StaticHeader::do_nothing)
            .map_err(|repair_error| format!("No valid StaticHeader found: {}", repair_error))?
            .ok_or_else(|| "No valid Stratis signature found".to_string())?;
    let bda = BDA::load(header, &mut devfile)
        .map_err(|bda_load_error| format!("BDA detected but error found: {}", bda_load_error))?
        .ok_or_else(|| "No Stratis BDA metadata found".to_string())?;

    println!("\n{:#?}", bda);

    devfile
        .seek(SeekFrom::Start(0))
        .map_err(|seek_err| format!("Error during seek: {}", seek_err))?;

    let loaded_state = bda
        .load_state(&mut devfile)
        .map_err(|stateload_err| format!("Error during load state: {}", stateload_err))?;

    println!("\nPool metadata:");
    if let Some(loaded_state) = loaded_state {
        let state_json: Value = serde_json::from_slice(&loaded_state)
            .map_err(|extract_err| format!("Error during state JSON extract: {}", extract_err))?;
        let state_json_pretty: String = serde_json::to_string_pretty(&state_json)
            .map_err(|parse_err| format!("Error during state JSON parse: {}", parse_err))?;
        println!("{}", state_json_pretty);
    } else {
        println!("None found");
    }

    Ok(())
}

fn main() {
    let matches = Command::new("stratis-dumpmetadata")
        .arg(
            Arg::new("dev")
                .required(true)
                .help("Print metadata of given device"),
        )
        .arg(
            Arg::new("print_bytes")
                .long("print-bytes")
                .short('b')
                .help("Print byte buffer of device"),
        )
        .get_matches();
    let devpath = matches.value_of("dev").unwrap();

    initialize_log();

    match run(devpath, matches.is_present("print_bytes")) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(1);
        }
    }
}
