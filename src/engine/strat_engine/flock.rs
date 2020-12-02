// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    os::unix::io::{AsRawFd, RawFd},
    path::{Path, PathBuf},
};

use nix::fcntl::{flock, FlockArg};

use crate::stratis::StratisResult;

#[allow(dead_code)]
pub enum DevFlockFlags {
    Exclusive,
    ExclusiveNonblock,
    Shared,
    SharedNonblock,
}

enum MaybeFile {
    File(File, PathBuf),
    Fd(RawFd),
}

pub struct DevFlock(MaybeFile);

impl DevFlock {
    #[allow(dead_code)]
    pub fn new(dev_path: &Path, flag: DevFlockFlags) -> StratisResult<DevFlock> {
        let file = File::open(dev_path)?;
        DevFlock::flock(file.as_raw_fd(), flag)?;
        Ok(DevFlock(MaybeFile::File(file, dev_path.to_owned())))
    }

    pub fn new_from_fd(fd: RawFd, flag: DevFlockFlags) -> StratisResult<DevFlock> {
        DevFlock::flock(fd, flag)?;
        Ok(DevFlock(MaybeFile::Fd(fd)))
    }

    fn flock(fd: RawFd, flag: DevFlockFlags) -> StratisResult<()> {
        Ok(flock(
            fd,
            match flag {
                DevFlockFlags::Exclusive => FlockArg::LockExclusive,
                DevFlockFlags::ExclusiveNonblock => FlockArg::LockExclusiveNonblock,
                DevFlockFlags::Shared => FlockArg::LockShared,
                DevFlockFlags::SharedNonblock => FlockArg::LockSharedNonblock,
            },
        )?)
    }
}

impl Drop for DevFlock {
    fn drop(&mut self) {
        if let MaybeFile::File(ref file, ref path) = self.0 {
            if let Err(e) = flock(file.as_raw_fd(), FlockArg::Unlock) {
                warn!(
                    "Failed to remove advisory lock on device {}: {}",
                    path.display(),
                    e,
                );
            }
        } else if let MaybeFile::Fd(fd) = self.0 {
            if let Err(e) = flock(fd, FlockArg::Unlock) {
                warn!(
                    "Failed to remove advisory lock on file descriptor {}: {}",
                    fd, e,
                );
            }
        }
    }
}

#[allow(dead_code)]
pub enum RecursiveFlock {
    Flock {
        lock: DevFlock,
        sublock: Box<RecursiveFlock>,
    },
    Base,
}

impl RecursiveFlock {
    #[allow(dead_code)]
    fn new(
        path: &Path,
        sublock: RecursiveFlock,
        flag: DevFlockFlags,
    ) -> StratisResult<RecursiveFlock> {
        Ok(RecursiveFlock::Flock {
            lock: DevFlock::new(path, flag)?,
            sublock: Box::new(sublock),
        })
    }

    #[allow(dead_code)]
    fn new_from_fd(
        fd: RawFd,
        sublock: RecursiveFlock,
        flag: DevFlockFlags,
    ) -> StratisResult<RecursiveFlock> {
        Ok(RecursiveFlock::Flock {
            lock: DevFlock::new_from_fd(fd, flag)?,
            sublock: Box::new(sublock),
        })
    }
}
