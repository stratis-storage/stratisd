// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate tempdir;

use libstratis::engine::strat_engine::blockdev::blkdev_size;
use libstratis::engine::strat_engine::blockdev::wipe_sectors;
use libstratis::engine::strat_engine::engine::DevOwnership;
use libstratis::engine::strat_engine::metadata::StaticHeader;
use libstratis::types::Sectors;

use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use util::blockdev_utils::tempdir::TempDir;
use util::test_result::TestError;
use util::test_result::TestErrorEnum;
use util::test_result::TestResult;

#[allow(dead_code)]
pub fn get_ownership(path: &Path) -> TestResult<DevOwnership> {

    let mut f = try!(OpenOptions::new()
        .read(true)
        .open(&path));

    let ownership = match StaticHeader::determine_ownership(&mut f) {
        Ok(ownership) => ownership,
        Err(err) => {
            let error_message = format!("{} for device {:?}", err, path);
            return Err(TestError::Framework(TestErrorEnum::Error(error_message)));
        }
    };

    Ok(ownership)
}

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

#[allow(dead_code)]
pub fn get_size(path: &Path) -> TestResult<Sectors> {
    let f = match File::open(path) {
        Ok(file) => file,
        Err(e) => panic!("Failed to open blockdev : {:?}", e),
    };

    match blkdev_size(&f) {
        Ok(bytes) => return Ok(bytes.sectors()),
        Err(e) => {
            let error_message = format!("{:?} for device {:?}", e, path);
            return Err(TestError::Framework(TestErrorEnum::Error(error_message)));
        }
    };
}

#[allow(dead_code)]
pub fn write_files_to_directory(tmp_dir: &TempDir, number_of_files: u32) -> TestResult<()> {
    for i in 0..number_of_files {
        {
            let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
            let mut tmp_file = File::create(file_path)
                .expect("failed to create temp file on filesystem");
            writeln!(tmp_file, "Write some data to file.").expect("failed to write temp file");
        }
    }
    Ok(())
}
