// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    collections::HashMap,
    error::Error,
    fs::OpenOptions,
    io::{self, Read, Write},
    path::Path,
};

use log::{set_logger, set_max_level, LevelFilter, Log, Metadata, Record};

use stratisd::systemd;

static LOGGER: SystemdLogger = SystemdLogger;

struct SystemdLogger;

impl Log for SystemdLogger {
    fn enabled(&self, _meta: &Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &Record<'_>) {
        systemd::syslog(record)
    }

    fn flush(&self) {}
}

pub fn setup_logger() -> Result<(), Box<dyn Error>> {
    set_logger(&LOGGER)?;
    set_max_level(LevelFilter::Info);
    Ok(())
}

pub fn get_kernel_cmdline() -> Result<HashMap<String, Option<Vec<String>>>, io::Error> {
    let mut cmdline = OpenOptions::new().read(true).open("/proc/cmdline")?;
    let mut cmdline_contents = String::new();
    cmdline.read_to_string(&mut cmdline_contents)?;

    let mut cmdline_map: HashMap<_, Option<Vec<String>>> = HashMap::new();
    for pair in cmdline_contents.split_whitespace() {
        let mut name_value = pair.splitn(2, '=');
        let name = name_value
            .next()
            .expect("Format must contain value")
            .to_string();
        let value_in_map = cmdline_map.get_mut(&name);
        let value = name_value.next().map(|s| s.to_string());
        match value_in_map {
            Some(Some(ref mut vec)) => {
                if let Some(v) = value {
                    vec.push(v);
                }
            }
            Some(val_in_map) => {
                if let Some(v) = value {
                    *val_in_map = Some(vec![v]);
                }
            }
            None => {
                cmdline_map.insert(name, value.map(|v| vec![v]));
            }
        }
    }
    Ok(cmdline_map)
}

pub fn write_unit_file(dest: &Path, file_contents: String) -> Result<(), io::Error> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dest)?;
    file.write_all(file_contents.as_bytes())?;
    Ok(())
}
