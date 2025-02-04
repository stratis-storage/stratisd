// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![cfg(all(feature = "engine", feature = "extras"))]

use assert_cmd::Command;
use predicates::prelude::predicate;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// stratisd-tools parser tests

#[test]
// Test stratisd-tools -V produces version string.
fn test_stratisd_tools_version() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-tools")?;
    cmd.arg("-V");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(VERSION));
    Ok(())
}

#[test]
// Test stratisd-tools when no subcommand is given.
fn test_stratisd_tools_no_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-tools")?;
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test that stratisd-tools rejects an unknown subcommand.
fn test_stratisd_tools_bad_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-tools")?;
    cmd.arg("notasub");
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test that stratisd-tools recognizes a good subcommand.
fn test_stratisd_tools_good_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratisd-tools")?;
    cmd.arg("stratis-dumpmetadata");
    cmd.arg("--help");
    cmd.assert().success();
    Ok(())
}
