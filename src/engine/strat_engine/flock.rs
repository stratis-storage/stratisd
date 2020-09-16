// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    os::unix::io::AsRawFd,
    path::{Path, PathBuf},
};

use nix::fcntl::{flock, FlockArg};

use crate::stratis::StratisResult;

#[allow(dead_code)]
pub enum DevFlockFlags {
    Ex,
    ExNB,
    Sh,
    ShNB,
}

pub struct DevFlock(File, PathBuf);

impl DevFlock {
    #[allow(dead_code)]
    pub fn new(dev_path: &Path, flag: DevFlockFlags) -> StratisResult<DevFlock> {
        let file = File::open(dev_path)?;
        flock(
            file.as_raw_fd(),
            match flag {
                DevFlockFlags::Ex => FlockArg::LockExclusive,
                DevFlockFlags::ExNB => FlockArg::LockExclusiveNonblock,
                DevFlockFlags::Sh => FlockArg::LockShared,
                DevFlockFlags::ShNB => FlockArg::LockSharedNonblock,
            },
        )?;
        Ok(DevFlock(file, dev_path.to_owned()))
    }
}

impl Drop for DevFlock {
    fn drop(&mut self) {
        if let Err(e) = flock(self.0.as_raw_fd(), FlockArg::Unlock) {
            warn!(
                "Failed to remove advisory lock on device {}: {}",
                self.1.display(),
                e,
            );
        }
    }
}
