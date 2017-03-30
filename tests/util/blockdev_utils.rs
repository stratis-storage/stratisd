// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate tempdir;

use libstratis::engine::strat_engine::device::wipe_sectors;
use libstratis::types::Sectors;

use std::path::Path;

use util::test_result::TestError;
use util::test_result::TestErrorEnum;
use util::test_result::TestResult;


pub fn clean_blockdev_headers(blockdev_paths: &[&Path]) -> TestResult<()> {

    for path in blockdev_paths {
        match wipe_sectors(path, Sectors(0), Sectors(16)) {
            Ok(_) => {}
            Err(e) => {
                let error_message = format!("{:?} for device {:?}", e, path);
                return Err(TestError::Framework(TestErrorEnum::Error(error_message)));
            }
        }
    }
    info!("devices cleaned for test");
    Ok(())
}
