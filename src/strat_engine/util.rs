// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::File;
use std::os::unix::prelude::AsRawFd;

use engine::{EngineResult, EngineError};

pub fn align_to(num: u64, align_to: u64) -> u64 {
    let agn = align_to - 1;

    (num + agn) & !agn
}

ioctl!(read blkgetsize64 with 0x12, 114; u64);

pub fn blkdev_size(file: &File) -> EngineResult<u64> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(EngineError::Nix(x)),
        Ok(_) => Ok(val),
    }
}

pub fn short_id(id: &str) -> String {
    let mut shortstr = id.to_owned();
    shortstr.truncate(8);
    shortstr
}
