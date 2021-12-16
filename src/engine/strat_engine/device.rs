// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{fs::File, os::unix::prelude::AsRawFd};

use devicemapper::Bytes;

use crate::stratis::{StratisError, StratisResult};

ioctl_read!(blkgetsize64, 0x12, 114, u64);

pub fn blkdev_size(file: &File) -> StratisResult<Bytes> {
    let mut val: u64 = 0;

    match unsafe { blkgetsize64(file.as_raw_fd(), &mut val) } {
        Err(x) => Err(StratisError::Nix(x)),
        Ok(_) => Ok(Bytes::from(val)),
    }
}
