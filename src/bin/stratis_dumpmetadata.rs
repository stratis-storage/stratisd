// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    env,
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    process,
    vec::Vec,
};

use env_logger::Builder;

use serde_json::Value;

use libstratis::engine::BDA;

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

fn run(devpath: &str) -> Result<(), String> {
    let mut devfile = OpenOptions::new()
        .read(true)
        .open(&devpath)
        .map_err(|the_io_error| format!("Error opening device: {}", the_io_error))?;

    let bda = BDA::load(&mut devfile)
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
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: stratis_dumpmetadata <device>");
        process::exit(2);
    }
    let devpath = &args[1];

    initialize_log();

    match run(devpath) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(1);
        }
    }
}
