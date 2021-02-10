// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    ops::{Deref, DerefMut},
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
    /// Flock the BlockDevPath recursively. This should be used for Stratis data
    /// structures respresenting devicemapper stacks.
    pub fn flock(&self, flag: DevFlockFlags) -> StratisResult<RecursiveFlock> {
        let mut locks = Vec::new();
        for path in self.child_paths.iter() {
            locks.push(path.flock(flag)?);
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

pub struct FlockRef<'a, T>(&'a T, RecursiveFlock);

impl<'a, T> Deref for FlockRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub struct FlockRefMut<'a, T>(&'a mut T, RecursiveFlock);

impl<'a, T> Deref for FlockRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<'a, T> DerefMut for FlockRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

/// A wrapper that maps a devicemapper data type's mutability constraints to the
/// appropriate type of flock arguments.
#[derive(Debug)]
pub struct WithFlock<T> {
    inner: T,
    path: Arc<BlockDevPath>,
}

impl<T> WithFlock<T> {
    /// Create a new wrapper.
    #[allow(dead_code)]
    pub fn new(inner: T, path: Arc<BlockDevPath>) -> Self {
        WithFlock { inner, path }
    }

    /// Create a shared flock and return an immutable reference.
    #[allow(dead_code)]
    pub fn flock(&self) -> StratisResult<FlockRef<'_, T>> {
        Ok(FlockRef(
            &self.inner,
            self.path.flock(DevFlockFlags::Shared)?,
        ))
    }

    /// Create an exclusive flock and return a mutable reference.
    #[allow(dead_code)]
    pub fn flock_mut(&mut self) -> StratisResult<FlockRefMut<'_, T>> {
        Ok(FlockRefMut(
            &mut self.inner,
            self.path.flock(DevFlockFlags::Exclusive)?,
        ))
    }

    /// Consume the wrapper and return the inner type.
    #[allow(dead_code)]
    pub fn into_inner(self) -> T {
        self.inner
    }
}
