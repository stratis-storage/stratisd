// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    process,
};

use clap::{App, Arg};

use serde_json::Value;

use libstratis::engine::{StaticHeader, StaticHeaderResult, BDA};

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
    println!(
        "{}",
        StaticHeaderResult::fmt_metadata(&read_results.0, print_bytes)
    );
    if read_results.0 != read_results.1 {
        println!(
            "Second sigblock differs: \n\n {}",
            StaticHeaderResult::fmt_metadata(&read_results.1, print_bytes)
        );
    }
    let header =
        StaticHeader::repair_sigblocks(&mut devfile, read_results, StaticHeader::do_nothing)
            .map_err(|repair_error| format!("No valid StaticHeader found: {}", repair_error))?
            .ok_or_else(|| "No valid Stratis signature found".to_string())?;
    let bda = BDA::load(header, &mut devfile)
        .map_err(|bda_load_error| format!("BDA detected but error found: {}", bda_load_error))?
        .ok_or_else(|| "No Stratis BDA metadata found".to_string())?;
    println!("{:#?}", bda);

    devfile
        .seek(SeekFrom::Start(0))
        .map_err(|seek_err| format!("Error during seek: {}", seek_err))?;

    let loaded_state = bda
        .load_state(&mut devfile)
        .map_err(|stateload_err| format!("Error during load state: {}", stateload_err))?;
    println!("Pool metadata:");
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
    let matches = App::new("stratis-dumpmetadata")
        .arg(
            Arg::with_name("dev")
                .required(true)
                .help("Print metadata of given device"),
        )
        .arg(Arg::with_name("print_bytes").help("Print byte buffer of device"))
        .get_matches();
    let devpath = matches.value_of("dev").unwrap();

    let print_bytes: bool = matches!(matches.value_of("print_bytes"), Some("bytes"));

    match run(devpath, print_bytes) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(1);
        }
    }
}
