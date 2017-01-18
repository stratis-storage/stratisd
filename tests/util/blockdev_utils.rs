// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
use libstratis::consts::SECTOR_SIZE;
use libstratis::engine::strat_engine::engine::DevOwnership;
use libstratis::engine::strat_engine::metadata::SigBlock;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use util::test_result::TestError;
use util::test_result::TestErrorEnum;

use util::test_result::TestResult;


#[allow(dead_code)]
pub fn get_ownership(path: &PathBuf) -> TestResult<DevOwnership> {

    let mut f = try!(OpenOptions::new()
        .read(true)
        .open(&path));

    let mut buf = [0u8; 4096];
    try!(f.seek(SeekFrom::Start(0)));
    try!(f.read(&mut buf));

    let ownership = match SigBlock::determine_ownership(&buf) {
        Ok(ownership) => ownership,
        Err(err) => {
            let error_message = format!("{} for device {:?}", err, path);
            return Err(TestError::Framework(TestErrorEnum::Error(error_message)));
        }
    };

    Ok(ownership)
}

pub fn clean_blockdev_headers(blockdev_paths: &Vec<&Path>) {

    for path in blockdev_paths {
        match wipe_header(path) {
            Ok(_) => {}
            Err(err) => {
                panic!("Failed to clean signature on {:?} : {:?}", path, err);
            }
        }
    }
    info!("devices cleaned for test");
}

fn wipe_header(path: &Path) -> TestResult<()> {
    let mut f = try!(OpenOptions::new().write(true).open(path));
    let zeroed = [0u8; (SECTOR_SIZE * 16) as usize];

    try!(f.write_all(&zeroed[..(SECTOR_SIZE * 16) as usize]));

    Ok(())
}
