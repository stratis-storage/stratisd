extern crate rustc_serialize;
extern crate stratisd;
extern crate devicemapper;
extern crate uuid;

#[macro_use]
extern crate log;
extern crate env_logger;

mod test_consts;
mod common;
mod pool_tests;
mod blockdev_tests;

use common::TestResult;
use std::path::Path;

fn log_test_result(test_name: &str, result: TestResult<()>) {
    match result {
        Ok(_) => trace!("PASSED: {} ", test_name),
        Err(e) => error!("FAILED: {}: {:?}", test_name, e),
    }
}

fn setup(blockdev_paths: &Vec<&Path>) {
    match common::clean_blockdevs_headers(blockdev_paths) {
        Ok(_) => {}
        Err(e) => error!("FAILED: clean_blockdevs_headers: {:?}", e),
    }
}

fn main() {
    env_logger::init().unwrap();

    info!("Starting Test Suite...");

    let mut config = match common::read_test_config(test_consts::DEFAULT_CONFIG_FILE) {
        Ok(config) => config,
        Err(_) => panic!(),
    };

    let safe_to_destroy_devs = match common::get_default_blockdevs(&mut config) {
        Ok(devs) => devs,
        Err(_) => panic!(),
    };


    setup(&safe_to_destroy_devs);
    let result = pool_tests::test_pools(&safe_to_destroy_devs);
    log_test_result("test_pools", result);

    setup(&safe_to_destroy_devs);
    let result = blockdev_tests::test_blockdevs(&safe_to_destroy_devs);
    log_test_result("test_blockdevs", result);

    info!("Test Suite Completed.");
}
