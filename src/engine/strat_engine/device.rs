// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::fs::File;

use iocuddle::{Group, Ioctl, Read};

use devicemapper::Bytes;

use crate::stratis::StratisResult;

const BLK: Group = Group::new(0x12);

const BLKGETSIZE64: Ioctl<Read, &u64> = unsafe { BLK.read(114) };

pub fn blkdev_size(file: &File) -> StratisResult<Bytes> {
    BLKGETSIZE64
        .ioctl(file)
        .map(|(_, res)| Bytes::from(res))
        .map_err(|e| e.into())
}
