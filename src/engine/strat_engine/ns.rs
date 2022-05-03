// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::create_dir_all,
    io,
    path::{Path, PathBuf},
};

use nix::{
    mount::{mount, umount, MsFlags},
    sched::{unshare, CloneFlags},
    sys::stat::stat,
    unistd::gettid,
};

use crate::stratis::{StratisError, StratisResult};

/// Path to the root mount namespace
const INIT_MNT_NS_PATH: &str = "/proc/1/ns/mnt";
/// Path to where private namespace mounts are mounted
pub const NS_TMPFS_LOCATION: &str = "/run/stratisd/ns_mounts";

pub fn unshare_namespace() -> StratisResult<()> {
    // Only create a new mount namespace if the thread is in the root namespace.
    if is_in_root_namespace()? {
        unshare(CloneFlags::CLONE_NEWNS)?;
    }
    assert!(!is_in_root_namespace()?);
    Ok(())
}

/// Check if the stratisd mount namespace for this thread is in the root namespace.
pub fn is_in_root_namespace() -> StratisResult<bool> {
    let pid_one_stat = stat(INIT_MNT_NS_PATH)?;
    let self_stat = stat(format!("/proc/self/task/{}/ns/mnt", gettid()).as_str())?;
    Ok(pid_one_stat.st_ino == self_stat.st_ino && pid_one_stat.st_dev == self_stat.st_dev)
}

/// A top-level tmpfs that can be made a private recursive mount so that any tmpfs
/// mounts inside of it will not be visible to any process but stratisd.
#[derive(Debug)]
pub struct MemoryFilesystem;

impl MemoryFilesystem {
    pub fn new() -> StratisResult<MemoryFilesystem> {
        let tmpfs_path = &Path::new(NS_TMPFS_LOCATION);
        if tmpfs_path.exists() {
            if !tmpfs_path.is_dir() {
                return Err(StratisError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a directory", tmpfs_path.display()),
                )));
            } else {
                let stat_info = stat(NS_TMPFS_LOCATION)?;
                let parent_path: PathBuf = vec![NS_TMPFS_LOCATION, ".."].iter().collect();
                let parent_stat_info = stat(&parent_path)?;
                if stat_info.st_dev != parent_stat_info.st_dev {
                    info!("Mount found at {}; unmounting", NS_TMPFS_LOCATION);
                    if let Err(e) = umount(NS_TMPFS_LOCATION) {
                        warn!(
                            "Failed to unmount filesystem at {}: {}",
                            NS_TMPFS_LOCATION, e
                        );
                    }
                }
            }
        } else {
            create_dir_all(NS_TMPFS_LOCATION)?;
        };
        mount(
            Some("tmpfs"),
            NS_TMPFS_LOCATION,
            Some("tmpfs"),
            MsFlags::empty(),
            Some("size=1M"),
        )?;

        mount::<str, str, str, str>(
            None,
            NS_TMPFS_LOCATION,
            None,
            MsFlags::MS_SLAVE | MsFlags::MS_REC,
            None,
        )?;
        Ok(MemoryFilesystem)
    }
}

impl Drop for MemoryFilesystem {
    fn drop(&mut self) {
        if let Err(e) = umount(NS_TMPFS_LOCATION) {
            warn!(
                "Could not unmount temporary in memory storage for private mounts: {}",
                e
            );
        }
    }
}
