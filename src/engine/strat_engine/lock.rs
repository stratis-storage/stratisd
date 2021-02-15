// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures::executor::block_on;
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

use crate::engine::types::BlockDevPath;

pub enum RecursiveLock<'a> {
    Lock {
        lock: RwLockReadGuard<'a, ()>,
        sublocks: Vec<RecursiveLock<'a>>,
        locked: Arc<AtomicBool>,
    },
    LockMut {
        lock: RwLockWriteGuard<'a, ()>,
        sublocks: Vec<RecursiveLock<'a>>,
        locked: Arc<AtomicBool>,
    },
    Base,
}

impl<'a> Drop for RecursiveLock<'a> {
    fn drop(&mut self) {
        match self {
            RecursiveLock::Lock { locked, .. } => locked.store(false, Ordering::Relaxed),
            RecursiveLock::LockMut { locked, .. } => locked.store(false, Ordering::Relaxed),
            RecursiveLock::Base => (),
        }
    }
}

impl BlockDevPath {
    /// Immutably lock the BlockDevPath recursively. This should be used for Stratis
    /// data structures respresenting devicemapper stacks.
    pub fn lock(&self) -> RecursiveLock {
        if self.locked.swap(true, Ordering::Relaxed) {
            return RecursiveLock::Base;
        }

        let mut sublocks = Vec::new();
        for path in self.child_paths.iter() {
            sublocks.push(path.lock());
        }
        RecursiveLock::Lock {
            lock: block_on(self.lock.read()),
            sublocks,
            locked: Arc::clone(&self.locked),
        }
    }

    /// Mutably lock the BlockDevPath recursively. This should be used for Stratis data
    /// structures respresenting devicemapper stacks.
    pub fn lock_mut(&self) -> RecursiveLock {
        if self.locked.swap(true, Ordering::Relaxed) {
            return RecursiveLock::Base;
        }

        let mut sublocks = Vec::new();
        for path in self.child_paths.iter() {
            sublocks.push(path.lock_mut());
        }
        RecursiveLock::LockMut {
            lock: block_on(self.lock.write()),
            sublocks,
            locked: Arc::clone(&self.locked),
        }
    }
}

pub struct LockRef<'a, T>(&'a T, RecursiveLock<'a>);

impl<'a, T> Deref for LockRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

pub struct LockRefMut<'a, T>(&'a mut T, RecursiveLock<'a>);

impl<'a, T> Deref for LockRefMut<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<'a, T> DerefMut for LockRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

/// A wrapper that maps a devicemapper data type's mutability constraints to the
/// appropriate type of flock arguments.
#[derive(Debug)]
pub struct WithLock<T> {
    inner: T,
    path: Arc<BlockDevPath>,
}

impl<T> WithLock<T> {
    /// Create a new wrapper.
    pub fn new(inner: T, path: Arc<BlockDevPath>) -> Self {
        WithLock { inner, path }
    }

    /// Create a shared flock and return an immutable reference.
    pub fn lock(&self) -> LockRef<'_, T> {
        LockRef(&self.inner, self.path.lock())
    }

    /// Create an exclusive flock and return a mutable reference.
    pub fn lock_mut(&mut self) -> LockRefMut<'_, T> {
        LockRefMut(&mut self.inner, self.path.lock_mut())
    }

    /// Consume the wrapper and return the inner type.
    pub fn into_inner(self) -> T {
        self.inner
    }
}
