// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Functions for dealing with devices.

use std::{fs::File, os::unix::prelude::AsRawFd, sync::LazyLock};

use libc::c_int;
use linux_raw_sys::ioctl::{BLKGETSIZE64, BLKPBSZGET, BLKSSZGET};

use devicemapper::Bytes;

use crate::stratis::{StratisError, StratisResult};

const NUM_SHIFT: u32 = linux_raw_sys::general::_IOC_NRSHIFT;
const TYPE_SHIFT: u32 = linux_raw_sys::general::_IOC_TYPEBITS;
const NUM_MASK: u32 = linux_raw_sys::general::_IOC_NRMASK;
const TYPE_MASK: u32 = linux_raw_sys::general::_IOC_TYPEMASK;

static BLKGETSIZE64_GROUP: LazyLock<u8> = LazyLock::new(|| {
    convert_int!((BLKGETSIZE64 >> TYPE_SHIFT) & TYPE_MASK, u32, u8).expect("Constant conversion")
});
static BLKGETSIZE64_NR: LazyLock<u8> = LazyLock::new(|| {
    convert_int!((BLKGETSIZE64 >> NUM_SHIFT) & NUM_MASK, u32, u8).expect("Constant conversion")
});

ioctl_read!(
    blkgetsize64,
    *LazyLock::force(&BLKGETSIZE64_GROUP),
    *LazyLock::force(&BLKGETSIZE64_NR),
    u64
);
ioctl_read_bad!(blksszget, BLKSSZGET, c_int);
ioctl_read_bad!(blkpbszget, BLKPBSZGET, c_int);

pub fn blkdev_size(file: &File) -> StratisResult<Bytes> {
    let mut val = 0u64; // util-linux uses int* as out-arg for ioctl
    unsafe { blkgetsize64(file.as_raw_fd(), &mut val) }.map_err(|e| {
        StratisError::Msg(format!(
            "Error reading logical sector size (BLKGETSIZE64): {e}"
        ))
    })?;
    Ok(Bytes::from(val))
}

pub fn blkdev_logical_sector_size(file: &File) -> StratisResult<Bytes> {
    let mut val = 0i32 as c_int; // util-linux uses int* as out-arg for ioctl
    unsafe { blksszget(file.as_raw_fd(), &mut val) }.map_err(|e| {
        StratisError::Msg(format!(
            "Error reading logical sector size (BLKSSZGET): {e}"
        ))
    })?;
    Ok(Bytes::from(convert_int!(val, c_int, u16)?))
}

pub fn blkdev_physical_sector_size(file: &File) -> StratisResult<Bytes> {
    let mut val = 0i32 as c_int; // util-linux uses int* as out-arg for ioctl
    unsafe { blkpbszget(file.as_raw_fd(), &mut val) }.map_err(|e| {
        StratisError::Msg(format!(
            "Error reading physical sector size (BLKPBSZGET): {e}"
        ))
    })?;
    Ok(Bytes::from(convert_int!(val, c_int, u16)?))
}
