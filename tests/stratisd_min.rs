// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
use assert_cmd::prelude::*;
use std::process::Command;
use std::{thread, time};

#[test]
fn test_stratisd_min_bad_loglevel() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-min")?;
    let assert = cmd.arg("--log-level").arg("nosuchlevel").assert();
    assert.failure().code(2);
    Ok(())
}

#[test]
fn test_stratisd_min_bad_option() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-min")?;
    let assert = cmd.arg("--nosim").assert();
    assert.failure().code(2);
    Ok(())
}

#[test]
fn test_stratisd_min_startup() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-min")?;
    let mut child = cmd.spawn()?;
    thread::sleep(time::Duration::from_secs(1));
    let mut cmd = Command::cargo_bin("stratis-min")?;
    cmd.arg("report").assert().success();
    child.kill()?;
    Ok(())
}
