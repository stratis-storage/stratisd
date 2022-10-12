// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{fs::File, os::unix::prelude::AsRawFd};

use iocuddle::{Group, Ioctl, Read};
use libc::c_int;

use devicemapper::Bytes;

use crate::stratis::{StratisError, StratisResult};

// BLKSSZGET is actually a Read ioctl which was accidentally defined
// using _IO. So, we must use the special _bad macro defined in nix.
// See: https://github.com/nix-rust/nix/issues/1006
// We have already tried and failed to use the available iocuddle functionality.
// The same holds true for BLKPBSZGET.
// If a new version of iocuddle were released, we could probably use the lie()
// function to get the correct functionality. See:
// https://github.com/stratis-storage/project/issues/533
ioctl_read_bad!(blksszget, 0x1268, c_int);
ioctl_read_bad!(blkpbszget, 0x127b, c_int);

const BLK: Group = Group::new(0x12);

const BLKGETSIZE64: Ioctl<Read, &u64> = unsafe { BLK.read(114) };

pub fn blkdev_size(file: &File) -> StratisResult<Bytes> {
    BLKGETSIZE64
        .ioctl(file)
        .map(|(_, res)| Bytes::from(res))
        .map_err(|e| e.into())
}

pub fn blkdev_logical_sector_size(file: &File) -> StratisResult<Bytes> {
    let mut val = 0i32 as c_int; // util-linux uses int* as out-arg for ioctl
    unsafe { blksszget(file.as_raw_fd(), &mut val) }.map_err(|e| {
        StratisError::Msg(format!(
            "Error reading logical sector size (BLKSSZGET): {}",
            e
        ))
    })?;
    // Allowed because the size should be less than u16::MAX
    #[allow(clippy::cast_possible_truncation)]
    Ok(Bytes::from(val as u16))
}

pub fn blkdev_physical_sector_size(file: &File) -> StratisResult<Bytes> {
    let mut val = 0i32 as c_int; // util-linux uses int* as out-arg for ioctl
    unsafe { blkpbszget(file.as_raw_fd(), &mut val) }.map_err(|e| {
        StratisError::Msg(format!(
            "Error reading physical sector size (BLKPBSZGET): {}",
            e
        ))
    })?;
    // Allowed because the size should be less than u16::MAX
    #[allow(clippy::cast_possible_truncation)]
    Ok(Bytes::from(val as u16))
}
