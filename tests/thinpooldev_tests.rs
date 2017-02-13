// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
#[macro_use]
extern crate log;
extern crate uuid;
extern crate devicemapper;
extern crate libstratis;
#[macro_use]
mod util;

use devicemapper::DM;

use libstratis::engine::strat_engine::blockdev;
use libstratis::engine::strat_engine::blockdev::BlockDev;
use libstratis::engine::strat_engine::metadata::MIN_MDA_SECTORS;
use libstratis::engine::strat_engine::thinpooldev::ThinPoolDev;
use libstratis::types::Sectors;

use std::iter::FromIterator;
use std::path::Path;
use std::thread;
use std::time::Duration;

use util::blockdev_utils::clean_blockdev_headers;
use util::blockdev_utils::get_size;
use util::test_config::TestConfig;
use util::test_consts::DEFAULT_CONFIG_FILE;
use util::test_result::TestError::Framework;
use util::test_result::TestErrorEnum::Error;
use util::test_result::TestResult;

use uuid::Uuid;

fn test_thinpool_setup(dm: &DM, blockdev_paths: &Vec<&Path>) -> TestResult<(ThinPoolDev)> {}

#[test]
pub fn test_thinpoolsetup_setup() {

    let dm = DM::new().unwrap();

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
    info!("devices cleaned for test");

    assert!(match test_thinpool_setup(&dm, &device_paths) {
        Ok(thinpool_dev) => {
            info!("Linear dev name : {:?}", thinpool_dev.name());
            let name = match thinpool_dev.name() {
                Ok(n) => n,
                Err(e) => panic!("Failed to get lineardev name {:?} ", e),
            };
            info!("completed test on {}", name);

            match thinpool_dev.teardown(&dm) {
                Ok(_) => info!("completed teardown of {}", name),
                Err(e) => panic!("Failed to teardown {} : {:?}", name, e),
            }
            true
        }
        Err(e) => {
            error!("Failed : test_lineardev_concat : {:?}", e);
            false
        }
    });
}
