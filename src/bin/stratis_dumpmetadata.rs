use std::env;

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::vec::Vec;

use serde_json::Value;

use libstratis::engine::strat_engine::backstore::metadata::BDA;

fn main() {
    let args: Vec<String> = env::args().collect();
    let devpath = args[1].clone();
    println!("Device path: {}", devpath);

    let mut devfile = OpenOptions::new().read(true).open(&devpath).unwrap();

    let bda = BDA::load(&mut devfile).unwrap().unwrap();
    println!("{:#?}", bda);

    devfile.seek(SeekFrom::Start(0)).unwrap();

    let loaded_state = bda.load_state(&mut devfile).unwrap();
    println!("State JSON data:");
    let state_json: Value = serde_json::from_slice(&loaded_state.unwrap()).unwrap();
    let state_json_pretty: String = serde_json::to_string_pretty(&state_json).unwrap();
    println!("{}", state_json_pretty);
}
