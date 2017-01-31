// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate libstratis;
extern crate devicemapper;
#[macro_use]
extern crate log;
#[macro_use]
mod util;

use libstratis::engine::Engine;
use libstratis::engine::strat_engine::StratEngine;
use libstratis::engine::strat_engine::engine::DevOwnership;

use std::path::Path;
use std::path::PathBuf;

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::get_ownership;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestResult;

// Check to make sure the disks in blockdev_paths are no longer "Owned" by
// Stratis
pub fn validate_disks_released(blockdev_paths: &Vec<PathBuf>) -> TestResult<()> {

    for path_name in blockdev_paths {
        let path = PathBuf::from(path_name);

        match get_ownership(&path) {
            Ok(ownership) => {
                match ownership {
                    DevOwnership::Unowned => {
                        debug!("Path confirmed {:?} Unowned | Theirs", path);
                    }
                    DevOwnership::Theirs => {
                        error!("Path set to Theirs after pool delete.  {:?}", path);
                        assert!(false);
                    }
                    DevOwnership::Ours(_) => {
                        error!("Failed to release {:?}", path);
                        assert!(false);
                    }
                }
            }
            Err(e) => {
                error!("Failed to determine ownership of {:?} {:?}", path, e);
                assert!(false);
            }
        }

    }

    Ok(())
}

// Validate that disks are "owned" by Stratis
pub fn validate_disks_init(blockdev_paths: &Vec<PathBuf>) -> TestResult<()> {

    for path_name in blockdev_paths {
        let path = PathBuf::from(path_name);

        match get_ownership(&path) {
            Ok(ownership) => {
                match ownership {
                    DevOwnership::Unowned | DevOwnership::Theirs => {
                        error!("Failed to initialize {:?}", path);
                        assert!(false)
                    }
                    DevOwnership::Ours(_) => {
                        debug!("Path {:?} is confirmed Ours.", path);
                    }
                }
            }
            Err(e) => {
                error!("Failed to determine ownership of {:?} {:?}", path, e);
                assert!(false)
            }
        }
    }

    Ok(())
}

pub fn test_create_and_delete(device_paths: &Vec<&Path>) -> TestResult<()> {

    let pool_name = "test_pool";

    let mut engine = StratEngine::new();

    let blockdevs = try!(engine.create_pool(pool_name, &device_paths, None, true));

    try!(validate_disks_init(&blockdevs));

    try!(engine.destroy_pool(pool_name));

    try!(validate_disks_released(&blockdevs));

    Ok(())
}

#[test]
pub fn test_pools() {
    let mut test_config = TestConfig::new(DEFAULT_CONFIG_FILE);

    let _ = test_config.init();

    let safe_to_destroy_devs = match test_config.get_safe_to_destroy_devs() {
        Ok(devs) => {
            if devs.len() == 0 {
                warn!("No devs availabe for testing.  Test not run");
                return;
            }
            devs
        }
        Err(e) => {
            error!("Failed : get_safe_to_destroy_devs : {:?}", e);
            return;
        }
    };
    info!("safe_to_destroy_devs = {:?}", safe_to_destroy_devs);

    let device_paths = safe_to_destroy_devs.iter().map(|x| Path::new(x)).collect::<Vec<&Path>>();

    clean_blockdev_headers(&device_paths);

    assert!(match test_create_and_delete(&device_paths) {
        Ok(_) => true,
        Err(_) => false,
    });

}
