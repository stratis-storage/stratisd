// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![cfg(all(feature = "dbus_enabled", feature = "engine"))]

use assert_cmd::Command;

#[test]
// Test stratis-utils menus.
fn test_stratis_utils_no_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test stratis-decode-dm without options.
fn test_stratis_utils_stratis_decode_dm() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm");
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test stratis-decode-dm with non-absolute path.
fn test_stratis_utils_stratis_decode_dm_non_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm")
        .arg("./dev/mapper/name")
        .arg("--output=filesystem-name");
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test stratis-decode-dm with non-absolute path.
fn test_stratis_utils_stratis_decode_dm_bad_output() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm")
        .arg("/dev/mapper/name")
        .arg("--output=filesystem-size");
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test stratis-decode-dm with no output mode specified.
fn test_stratis_utils_stratis_decode_dm_no_output() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm").arg("/dev/mapper/name");
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test stratis-decode-dm with bad absolute path.
fn test_stratis_utils_stratis_decode_dm_bad_path() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm")
        .arg("/dev/scrapper/name")
        .arg("--output=filesystem-name");
    cmd.assert().failure().code(1);
    Ok(())
}

#[test]
// Test stratis-decode-dm with unparsable device name.
fn test_stratis_utils_stratis_decode_dm_bad_name() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm")
        .arg("/dev/mapper/name")
        .arg("--output=filesystem-name");
    cmd.assert().failure().code(1);
    Ok(())
}

#[test]
// Test stratis-decode-dm with out stratisd running
fn test_stratis_utils_stratis_decode_dm_no_stratisd() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-utils")?;
    cmd.arg("stratis-decode-dm")
        .arg("/dev/mapper/stratis-1-824da802f37d43c7916f71d33fc9a208-thin-fs-bc5302fd8bc6405f8f98b39ecf13c088")
        .arg("--output=filesystem-name");
    cmd.assert().failure().code(1);
    Ok(())
}
