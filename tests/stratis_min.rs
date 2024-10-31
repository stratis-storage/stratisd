// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![cfg(all(feature = "engine", feature = "min"))]

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
    cmd.assert().failure().code(2);
    Ok(())
}

#[test]
// Test that stratis-min rejects an unknown subcommand.
fn test_stratis_min_bad_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin("stratis-min")?;
    cmd.arg("notasub");
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));
    Ok(())
}

#[test]
// Test that stratis-min report fails when no daemon is running.
fn test_stratis_min_report_no_daemon() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("report");
    cmd.assert()
        .failure()
        .code(1)
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
        .code(2)
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
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "'--thumbprint <thumbprint>' cannot be used with '--trust-url'",
        ));
}

// Test parsing when creating a pool w/ clevis tang, and a URL.
fn stratis_min_create_with_clevis_url() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("tang")
        .arg("--tang-url")
        .arg("url")
        .arg("--trust-url")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().success();
}

#[test]
fn test_stratis_min_create_with_clevis_url() {
    test_with_stratisd_min_sim(stratis_min_create_with_clevis_url);
}

// Test parsing when creating a pool w/ clevis TPM2.
fn stratis_min_create_with_clevis_tpm() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("create")
        .arg("--clevis")
        .arg("tpm2")
        .arg("pn")
        .arg("/dev/n");
    cmd.assert().success();
}

#[test]
fn test_stratis_min_create_with_clevis_tpm() {
    test_with_stratisd_min_sim(stratis_min_create_with_clevis_tpm);
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
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
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
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
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
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ));
}

#[test]
// Test parsing when creating a pool with no blockdevs.
fn test_stratis_min_create_no_blockdevs() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("create").arg("pn");
    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "required arguments were not provided",
        ));
}

#[test]
// Test stopping a pool using an invalid UUID; unless name is specified the
// id value is interpreted as a UUID.
fn test_stratis_min_pool_stop_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("stop").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test starting a pool using an invalid UUID; unless name is specified the
// id value is interpreted as a UUID.
fn test_stratis_min_pool_start_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("start").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test starting a pool using an invalid unlock method.
fn test_stratis_min_pool_start_invalid_unlock_method() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("start")
        .arg("--name")
        .arg("pn")
        .arg("--unlock-method=bogus");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "bogus is an invalid unlock method",
        ));
}

#[test]
// Test binding a pool using an invalid UUID; unless name is specified the
// id value is interpreted as a UUID.
fn test_stratis_min_pool_bind_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("bind")
        .arg("keyring")
        .arg("pn")
        .arg("--key-desc")
        .arg("desc");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("bind")
        .arg("tang")
        .arg("pn")
        .arg("http://abc")
        .arg("--trust-url");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("bind").arg("tpm2").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test unbinding a pool using an invalid UUID; unless name is specified the
// id value is interpreted as a UUID.
fn test_stratis_min_pool_unbind_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("unbind").arg("keyring").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("unbind").arg("clevis").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test rebinding a pool using an invalid UUID; unless name is specified the
// id value is interpreted as a UUID.
fn test_stratis_min_pool_rebind_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("rebind")
        .arg("keyring")
        .arg("--key-desc")
        .arg("desc")
        .arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("rebind").arg("clevis").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test checking a pool property using an invalid UUID; unless name is
// specified the id value is interpreted as a UUID.
fn test_stratis_min_pool_properties_invalid_uuid() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-encrypted").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-stopped").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-bound").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("has-passphrase").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));

    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("clevis-pin").arg("pn");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Uuid error"));
}

#[test]
// Test running "stratis pool bind" with missing subcommand.
fn test_stratis_min_pool_bind_missing_subcommand() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("bind");
    cmd.assert().failure().code(2);
}

#[test]
// Test running "stratis pool unbind" with missing subcommand.
fn test_stratis_min_pool_unbind_missing_subcommand() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("unbind");
    cmd.assert().failure().code(2);
}

#[test]
// Test running "stratis pool rebind" with missing subcommand.
fn test_stratis_min_pool_rebind_missing_subcommand() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("rebind");
    cmd.assert().failure().code(2);
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
        .code(1)
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

fn stratis_min_fs_rename() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("filesystem")
        .arg("rename")
        .arg("pn")
        .arg("fn")
        .arg("fm");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("filesystem")
        .assert()
        .success()
        .stdout(predicate::str::contains("fm"));
}

#[test]
// Test renaming a filesystem.
fn test_stratis_min_fs_rename() {
    test_with_stratisd_min_sim(stratis_min_fs_rename);
}

fn stratis_min_pool_stop_name() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("stop").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
// Test stopping a pool using a valid name.
fn test_stratis_min_pool_stop_name() {
    test_with_stratisd_min_sim(stratis_min_pool_stop_name);
}

fn stratis_min_pool_add_data() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("add-data")
        .arg("pn")
        .arg("/dev/nonexistentblockdev1");
    cmd.assert().success();
}

#[test]
// Test adding a data device to a pool.
fn test_stratis_min_pool_add_data() {
    test_with_stratisd_min_sim(stratis_min_pool_add_data);
}

fn stratis_min_pool_add_cache() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("init-cache")
        .arg("pn")
        .arg("/dev/nonexistentblockdev1");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("add-cache")
        .arg("pn")
        .arg("/dev/nonexistentblockdev2");
    cmd.assert().success();
}

#[test]
// Test adding a cache device to a pool.
fn test_stratis_min_pool_add_cache() {
    test_with_stratisd_min_sim(stratis_min_pool_add_cache);
}

fn stratis_min_pool_stop_start_name() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("stop").arg("--name").arg("pn");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("start").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
// Test stopping and starting a pool using a valid name.
fn test_stratis_min_pool_stop_start_name() {
    test_with_stratisd_min_sim(stratis_min_pool_stop_start_name);
}

fn stratis_min_pool_is_encrypted() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-encrypted").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
// Test if a pool is encrypted.
fn test_stratis_min_pool_is_encrypted() {
    test_with_stratisd_min_sim(stratis_min_pool_is_encrypted);
}

fn stratis_min_pool_is_stopped() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-stopped").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
// Test if a pool is stopped.
fn test_stratis_min_pool_is_stopped() {
    test_with_stratisd_min_sim(stratis_min_pool_is_stopped);
}

fn stratis_min_pool_is_bound() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("is-bound").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
// Test if a pool is bound.
fn test_stratis_min_pool_is_bound() {
    test_with_stratisd_min_sim(stratis_min_pool_is_bound);
}

fn stratis_min_pool_has_passphrase() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("has-passphrase")
        .arg("--name")
        .arg("pn");
    cmd.assert().success();
}

#[test]
// Test if a pool has a passphrase.
fn test_stratis_min_pool_has_passphrase() {
    test_with_stratisd_min_sim(stratis_min_pool_has_passphrase);
}

fn stratis_min_pool_stop_nonexistent_uuid() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool")
        .arg("stop")
        .arg("44444444444444444444444444444444");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "was not found and cannot be stopped",
        ));
}

#[test]
// Test trying to stop a pool with a nonexistent UUID.
fn test_stratis_min_pool_stop_nonexistent_uuid() {
    test_with_stratisd_min_sim(stratis_min_pool_stop_nonexistent_uuid);
}

fn stratis_min_fs_origin() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("filesystem").arg("origin").arg("pn").arg("fn");
    cmd.assert().success();
}

#[test]
fn test_stratis_min_fs_origin() {
    test_with_stratisd_min_sim(stratis_min_fs_origin);
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

fn stratis_min_key_set() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.write_stdin("thisisatestpassphrase\n")
        .arg("key")
        .arg("set")
        .arg("--capture-key")
        .arg("testkey");
    cmd.assert().success();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("key").arg("unset").arg("testkey");
    cmd.assert().success();
}

#[test]
fn test_stratis_min_key_set() {
    test_with_stratisd_min_sim(stratis_min_key_set);
}

fn stratis_min_key_set_empty() {
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.write_stdin("")
        .arg("key")
        .arg("set")
        .arg("--capture-key")
        .arg("testkey");
    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("Password provided was empty"));
}

#[test]
fn test_stratis_min_key_set_empty() {
    test_with_stratisd_min_sim(stratis_min_key_set_empty);
}

fn stratis_min_pool_clevis_pin() {
    stratis_min_create_pool_and_fs();
    let mut cmd = Command::cargo_bin("stratis-min").unwrap();
    cmd.arg("pool").arg("clevis-pin").arg("--name").arg("pn");
    cmd.assert().success();
}

#[test]
fn test_stratis_min_pool_clevis_pin() {
    test_with_stratisd_min_sim(stratis_min_pool_clevis_pin);
}
