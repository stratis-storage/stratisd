// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
use assert_cmd::Command;
use common::test_with_stratisd_min_sim;
use predicates::prelude::predicate;

mod common;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// stratis-min parser tests

#[test]
// Test stratis-min -V produces version string.
fn test_stratis_min_version() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-min")?;
    cmd.arg("-V");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(VERSION));
    Ok(())
}

#[test]
// Test stratis-min when no subcommand is given.
fn test_stratis_min_no_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-min")?;
    let assert = cmd.assert();
    assert.failure().code(2);
    Ok(())
}

#[test]
// Test that stratis-min rejects an unknown subcommand.
fn test_stratis_min_bad_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-min")?;
    let assert = cmd.arg("notasub").assert();
    assert.failure().code(2);
    Ok(())
}

#[test]
// Test that stratis-min report fails when no daemon is running.
fn test_stratis_min_report_no_daemon() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("report");
    cmd.assert()
        .failure()
        .stderr(predicates::str::contains("IO error"));
}

#[test]
// Test that stratis-min fails if given a report type.
fn test_stratis_min_report_bad_subreport() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    // stratis min does not accept report type.
    cmd.arg("report").arg("stopped_pools");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
// Test parsing when creating a pool w/ clevis tang, a URL, but both
// thumbprint and --trust-url set.
fn test_stratis_min_create_with_clevis_1() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("tang")
        .arg("--tang-url")
        .arg("url")
        .arg("--thumbprint")
        .arg("jkj")
        .arg("--trust-url")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().failure().stderr(predicate::str::contains(
        "'--thumbprint <thumbprint>' cannot be used with '--trust-url'",
    ));
}

#[test]
// Test parsing when creating a pool with an invalid Clevis method.
fn test_stratis_min_create_with_clevis_invalid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("nosuch")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().failure().stderr(predicate::str::contains(
        "invalid value 'nosuch' for '--clevis <clevis>'",
    ));
}

#[test]
// Test parsing when creating a pool with Clevis and missing tang
// arguments.
fn test_stratis_min_create_with_clevis_missing_args() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("tang")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().failure().stderr(predicate::str::contains(
        "required arguments were not provided",
    ));
}

#[test]
// Test parsing when creating a pool with Clevis and Tang URL
// but no thumbprint or trust-url.
fn test_stratis_min_create_with_clevis_invalid_2() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("tang")
        .arg("--tang-url")
        .arg("url")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().failure().stderr(predicate::str::contains(
        "required arguments were not provided",
    ));
}

#[test]
// Test parsing when creating a pool with no blockdevs.
fn test_stratis_min_create_no_blockdevs() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("create").arg("pn");
    cmd.assert().failure().stderr(predicate::str::contains(
        "required arguments were not provided",
    ));
}

// stratis-min tests with sim engine

fn stratis_min_create_pool_and_fs() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("create").arg("pn").arg("/dev/n");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("filesystem").arg("create").arg("pn").arg("fn");
    cmd.assert().success();
}

#[test]
// Test that creating a pool and filesystem succeeds with the
// simulator engine.
fn test_stratis_min_create() {
    test_with_stratisd_min_sim(stratis_min_create_pool_and_fs);
}

fn stratis_min_create_destroy() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("filesystem").arg("destroy").arg("pn").arg("fn");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("destroy").arg("pn");
    cmd.assert().success();
}

#[test]
// Test that creating and destroying a pool and filesystem
// succeeds with the simulator engine.
fn test_stratis_min_create_destroy() {
    test_with_stratisd_min_sim(stratis_min_create_destroy);
}

fn stratis_min_destroy_with_fs() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("destroy").arg("pn");
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("filesystems remaining on pool"));
}

#[test]
// Test that destroying a pool containing a filesystem fails
// with the simulator engine.
fn test_stratis_min_destroy_with_fs() {
    test_with_stratisd_min_sim(stratis_min_destroy_with_fs);
}

fn stratis_min_pool_rename() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("rename").arg("pn").arg("pm");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .assert()
        .success()
        .stdout(predicate::str::contains("pm"));
}

#[test]
// Test that renaming a pool succeeds and that the new name is
// present in stratis-min pool output.
fn test_stratis_min_pool_rename() {
    test_with_stratisd_min_sim(stratis_min_pool_rename);
}

fn stratis_min_report() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("report");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("pools"));
}

#[test]
// Test that generating a report with the simulator engine
// succeeds and contains the expected "pools" key.
fn test_stratis_min_report() {
    test_with_stratisd_min_sim(stratis_min_report);
}

fn stratis_min_list_default() {
    let subcommands = [
        ("pool", "Name"),
        ("filesystem", "Pool Name"),
        ("key", "Key Description"),
    ];
    for (sc, expect) in subcommands.iter() {
        let mut cmd = Command::cargo_bin("stratis-min").unwrap();
        cmd.arg(sc)
            .assert()
            .success()
            .stdout(predicate::str::contains(*expect));
    }
}

#[test]
// Verify that pool, filesystem, and key subcommands execute
// without any additional command and produce the expected
// report headings.
fn test_stratis_min_list_defaults() {
    test_with_stratisd_min_sim(stratis_min_list_default);
}
