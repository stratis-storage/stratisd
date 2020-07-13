use std::{
    env,
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    process,
    vec::Vec,
};

use serde_json::Value;

use libstratis::engine::BDA;

fn run(devpath: String) -> Result<(), String> {
    let mut devfile = OpenOptions::new()
        .read(true)
        .open(&devpath)
        .map_err(|_the_io_error| format!("Error opening device: {}", _the_io_error))?;

    let bda = BDA::load(&mut devfile)
        .map_err(|bda_load_error| format!("BDA detected but error found: {}", bda_load_error))?
        .ok_or_else(|| "No Stratis BDA metadata found".to_string())?;
    println!("{:#?}", bda);

    devfile.seek(SeekFrom::Start(0)).unwrap();

    let loaded_state = bda.load_state(&mut devfile).unwrap();
    println!("State JSON data:");
    let state_json: Value = serde_json::from_slice(&loaded_state.unwrap()).unwrap();
    let state_json_pretty: String = serde_json::to_string_pretty(&state_json).unwrap();
    println!("{}", state_json_pretty);
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: stratis_dumpmetadata <device>");
        process::exit(1);
    }
    let devpath = args[1].clone();

    match run(devpath) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error encountered: {}", e);
            process::exit(2);
        }
    }
}
