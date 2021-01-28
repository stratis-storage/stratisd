// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    os::unix::io::{AsRawFd, RawFd},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use nix::fcntl::{flock, FlockArg};

use crate::{engine::types::BlockDevPath, stratis::StratisResult};

#[derive(Copy, Clone)]
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

pub struct DevFlock(MaybeFile, Arc<AtomicBool>);

impl DevFlock {
    #[allow(dead_code)]
    pub fn new(
        dev_path: &Path,
        locked: Arc<AtomicBool>,
        flag: DevFlockFlags,
    ) -> StratisResult<Option<DevFlock>> {
        if locked.swap(true, Ordering::Relaxed) {
            return Ok(None);
        }

        let file = File::open(dev_path)?;
        DevFlock::flock(file.as_raw_fd(), flag)?;
        Ok(Some(DevFlock(
            MaybeFile::File(file, dev_path.to_owned()),
            locked,
        )))
    }

    pub fn new_from_fd(
        fd: RawFd,
        locked: Arc<AtomicBool>,
        flag: DevFlockFlags,
    ) -> StratisResult<Option<DevFlock>> {
        if locked.swap(true, Ordering::Relaxed) {
            return Ok(None);
        }

        DevFlock::flock(fd, flag)?;
        Ok(Some(DevFlock(MaybeFile::Fd(fd), locked)))
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
        self.1.store(false, Ordering::Relaxed);
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
        sublocks: Vec<RecursiveFlock>,
    },
    Base,
}

impl RecursiveFlock {
    #[allow(dead_code)]
    fn new(
        path: &Path,
        locked: Arc<AtomicBool>,
        sublocks: Vec<RecursiveFlock>,
        flag: DevFlockFlags,
    ) -> StratisResult<RecursiveFlock> {
        let lock = match DevFlock::new(path, locked, flag)? {
            Some(flock) => flock,
            None => return Ok(RecursiveFlock::Base),
        };
        Ok(RecursiveFlock::Flock { lock, sublocks })
    }

    #[allow(dead_code)]
    fn new_from_fd(
        fd: RawFd,
        locked: Arc<AtomicBool>,
        sublocks: Vec<RecursiveFlock>,
        flag: DevFlockFlags,
    ) -> StratisResult<RecursiveFlock> {
        let lock = match DevFlock::new_from_fd(fd, locked, flag)? {
            Some(flock) => flock,
            None => return Ok(RecursiveFlock::Base),
        };
        Ok(RecursiveFlock::Flock { lock, sublocks })
    }
}

impl BlockDevPath {
    /// Flock this single BlockDevPath. This should be used for metadata operations
    /// on Stratis devices prior to the data structures being set up.
    pub fn flock(&self, flag: DevFlockFlags) -> StratisResult<Option<DevFlock>> {
        DevFlock::new(&self.path, Arc::clone(&self.locked), flag)
    }

    /// Flock the BlockDevPath recursively. This should be used for Stratis data
    /// structures respresenting devicemapper stacks.
    pub fn rec_flock(&self, flag: DevFlockFlags) -> StratisResult<RecursiveFlock> {
        let mut locks = Vec::new();
        for path in self.child_paths.iter() {
            locks.push(path.rec_flock(flag)?);
        }
        RecursiveFlock::new(
            &self.path,
            Arc::clone(&self.locked),
            if locks.is_empty() {
                vec![RecursiveFlock::Base]
            } else {
                locks
            },
            flag,
        )
    }
}
