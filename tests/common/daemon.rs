// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
use assert_cmd::prelude::CommandCargoExt;
use std::{
    panic,
    process::{Child, Command},
    thread, time,
};

use crate::common::logger::init_logger;

fn start_stratisd_min(sim: bool) -> Result<Child, Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-min")?;
    if sim {
        cmd.arg("--sim");
    }
    let child = cmd.spawn().expect("stratisd-min failed to start");
    thread::sleep(time::Duration::from_secs(1));
    Ok(child)
}

fn stop_stratisd_min(mut daemon: Child) -> Result<(), Box<dyn std::error::Error>> {
    daemon.kill()?;
    daemon.wait()?;
    Ok(())
}

// Run a test with stratisd-min
pub fn test_with_stratisd_min_sim<F>(test: F)
where
    F: Fn() + panic::RefUnwindSafe,
{
    init_logger();
    let daemon = start_stratisd_min(true).unwrap();

    let result = panic::catch_unwind(|| {
        test();
    });
    let td = stop_stratisd_min(daemon);

    result.unwrap();
    td.unwrap();
}
