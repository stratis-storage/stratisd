use std::{
    env,
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    process,
    vec::Vec,
};

use serde_json::Value;

use libstratis::engine::BDA;

fn run(devpath: &str) -> Result<(), String> {
    let mut devfile = OpenOptions::new()
        .read(true)
        .open(&devpath)
        .map_err(|the_io_error| format!("Error opening device: {}", the_io_error))?;

    let bda = BDA::load(&mut devfile)
        .map_err(|bda_load_error| format!("BDA detected but error found: {}", bda_load_error))?
        .ok_or_else(|| "No Stratis BDA metadata found".to_string())?;
    println!("{:#?}", bda);

    devfile.seek(SeekFrom::Start(0)).unwrap();

    let loaded_state = bda
        .load_state(&mut devfile)
        .map_err(|stateload_err| format!("Error during load state: {}", stateload_err))?;
    println!("State JSON data:");

    let state_json: Value = serde_json::from_slice(&loaded_state.unwrap())
        .map_err(|extract_err| format!("Error during state JSON extract: {}", extract_err))?;
    let state_json_pretty: String = serde_json::to_string_pretty(&state_json)
        .map_err(|parse_err| format!("Error during state JSON parse: {}", parse_err))?;
    println!("{}", state_json_pretty);

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: stratis_dumpmetadata <device>");
        process::exit(2);
    }
    let devpath = &args[1];

    match run(devpath) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(1);
        }
    }
}
