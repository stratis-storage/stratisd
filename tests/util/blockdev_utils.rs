// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
extern crate tempdir;

use libstratis::consts::SECTOR_SIZE;
use libstratis::engine::strat_engine::device::blkdev_size;
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
        match wipe_header(path) {
            Ok(_) => {}
            Err(e) => {
                error!("Failed to clean signature on {:?} : {:?}", path, e);
                return Err(e);
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

pub fn wipe_header(path: &Path) -> TestResult<()> {
    let mut f = try!(OpenOptions::new().write(true).open(path));
    let zeroed = [0u8; SECTOR_SIZE * 16];

    try!(f.write_all(&zeroed[..SECTOR_SIZE * 16]));

    Ok(())
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
