// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    fs::File,
    path::{Path, PathBuf},
};

use nix::mount::{umount2, MntFlags};
use retry::{delay::Fixed, retry};

use devicemapper::{DevId, DmFlags, DmName, DmNameBuf, DmOptions, DM};

use crate::{
    engine::strat_engine::{
        cmd::udev_settle,
        device::blkdev_size,
        dm::{get_dm, get_dm_init},
    },
    stratis::StratisResult,
};

mod cleanup_errors {
    use std::fmt;

    use devicemapper::DmError;

    #[derive(Debug)]
    pub enum Error {
        Ioe(std::io::Error),
        Nix(nix::Error),
        Msg(String),
        Chained(String, Box<Error>),
        Dm(DmError),
        Procfs(procfs::ProcError),
    }

    pub type Result<T> = std::result::Result<T, Error>;

    impl From<nix::Error> for Error {
        fn from(err: nix::Error) -> Error {
            Error::Nix(err)
        }
    }

    impl From<std::io::Error> for Error {
        fn from(err: std::io::Error) -> Error {
            Error::Ioe(err)
        }
    }

    impl From<String> for Error {
        fn from(err: String) -> Error {
            Error::Msg(err)
        }
    }

    impl From<DmError> for Error {
        fn from(err: DmError) -> Error {
            Error::Dm(err)
        }
    }

    impl From<procfs::ProcError> for Error {
        fn from(err: procfs::ProcError) -> Error {
            Error::Procfs(err)
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::Ioe(err) => write!(f, "IO error: {err}"),
                Error::Nix(err) => write!(f, "Nix error: {err}"),
                Error::Msg(err) => write!(f, "{err}"),
                Error::Chained(msg, err) => write!(f, "{msg}: {err}"),
                Error::Dm(err) => write!(f, "DM error: {err}"),
                Error::Procfs(err) => write!(f, "Procfs error: {err}"),
            }
        }
    }

    impl std::error::Error for Error {}
}

use self::cleanup_errors::{Error, Result};

/// Attempt to remove all device mapper devices which match the stratis naming convention.
/// FIXME: Current implementation complicated by https://bugzilla.redhat.com/show_bug.cgi?id=1506287
pub fn dm_stratis_devices_remove() -> Result<()> {
    /// One iteration of removing devicemapper devices
    fn one_iteration() -> Result<Vec<DmNameBuf>> {
        #[allow(clippy::to_string_in_format_args)]
        get_dm()
            .list_devices()
            .map_err(|e| {
                Error::Chained(
                    "failed while listing DM devices, giving up".into(),
                    Box::new(e.into()),
                )
            })
            .map(|devices| {
                devices
                    .iter()
                    .map(|d| &d.0)
                    .filter_map(|n| {
                        if !n.to_string().starts_with("stratis-1")
                            && !n.to_string().starts_with("stratis_fail_device")
                            && !n.to_string().starts_with("stratis_test_device")
                        {
                            None
                        } else if let Err(retry::Error { error, .. }) =
                            retry(Fixed::from_millis(1000).take(3), || {
                                get_dm().device_remove(&DevId::Name(n), DmOptions::default())
                            })
                        {
                            debug!("Failed to remove device {}: {}", n.to_string(), error);
                            Some(n.to_owned())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
    }

    /// Do one iteration of removals until progress stops. Return remaining
    /// dm devices.
    fn do_while_progress() -> Result<Vec<DmNameBuf>> {
        let mut remaining = one_iteration()?;
        while !remaining.is_empty() {
            let temp = one_iteration()?;
            if temp.len() < remaining.len() {
                remaining = temp;
            } else {
                break;
            }
        }
        Ok(remaining)
    }

    || -> Result<()> {
        udev_settle().unwrap();
        get_dm_init().map_err(|_| Error::Msg("Unable to initialize DM".into()))?;
        do_while_progress().and_then(|remain| {
            if !remain.is_empty() {
                Err(format!("Some Stratis DM devices remaining: {remain:?}").into())
            } else {
                Ok(())
            }
        })
    }()
    .map_err(|e| {
        Error::Chained(
            "Failed to ensure removal of all test-generated DM devices".into(),
            Box::new(e),
        )
    })
}

/// Try and un-mount any filesystems that have the name stratis in the mount point, returning
/// immediately on the first one we are unable to unmount.
fn stratis_filesystems_unmount() -> Result<()> {
    || -> Result<()> {
        for mount_point in procfs::process::Process::myself()?
            .mountinfo()?
            .into_iter()
            .map(|i| i.mount_point)
            .filter(|mp| mp.as_path().to_string_lossy().contains("stratis"))
        {
            umount2(&mount_point, MntFlags::MNT_DETACH)?;
        }
        Ok(())
    }()
    .map_err(|e| {
        Error::Chained(
            "Failed to ensure all Stratis filesystems were unmounted".into(),
            Box::new(e),
        )
    })
}

/// When a unit test panics we can leave the system in an inconsistent state.  This function
/// tries to clean up by un-mounting any mounted file systems which contain the string
/// "stratis_testing" and then it tries to remove any device mapper tables which are also stratis
/// created.
pub fn clean_up() -> Result<()> {
    stratis_filesystems_unmount().and_then(|_| dm_stratis_devices_remove())
}

pub struct FailDevice {
    backing_device: PathBuf,
    test_device_name: String,
    dm_context: DM,
    size: u64,
}

impl FailDevice {
    pub fn new(backing_device: &Path, test_device_name: &str) -> StratisResult<Self> {
        let dm = DM::new()?;
        let dm_name = DmName::new(test_device_name)?;
        let dev_id = DevId::Name(dm_name);

        let size = {
            let file = File::open(backing_device)?;
            blkdev_size(&file)?
        };

        dm.device_create(dm_name, None, DmOptions::default())?;
        dm.table_load(
            &dev_id,
            &[(
                0,
                *size.sectors(),
                "linear".to_string(),
                format!("{} 0", backing_device.display()),
            )],
            DmOptions::default(),
        )?;
        dm.device_suspend(&dev_id, DmOptions::default())?;

        Ok(FailDevice {
            backing_device: backing_device.to_owned(),
            test_device_name: test_device_name.to_owned(),
            dm_context: dm,
            size: *size.sectors(),
        })
    }

    pub fn as_path(&self) -> PathBuf {
        vec!["/dev/mapper", self.test_device_name.as_str()]
            .into_iter()
            .collect::<PathBuf>()
    }

    pub fn start_failing(&self, num_sectors_after_start: u64) -> StratisResult<()> {
        let dm_name = DmName::new(self.test_device_name.as_str())?;
        let dev_id = DevId::Name(dm_name);

        self.dm_context
            .device_suspend(&dev_id, DmOptions::default().set_flags(DmFlags::DM_SUSPEND))?;
        self.dm_context.table_load(
            &dev_id,
            &[
                (
                    0,
                    num_sectors_after_start,
                    "error".to_string(),
                    String::new(),
                ),
                (
                    num_sectors_after_start,
                    self.size - num_sectors_after_start,
                    "linear".to_string(),
                    format!(
                        "{} {}",
                        self.backing_device.display(),
                        num_sectors_after_start
                    ),
                ),
            ],
            DmOptions::default(),
        )?;
        self.dm_context
            .device_suspend(&dev_id, DmOptions::default())?;

        Ok(())
    }

    pub fn stop_failing(&self) -> StratisResult<()> {
        let dm_name = DmName::new(self.test_device_name.as_str())?;
        let dev_id = DevId::Name(dm_name);

        self.dm_context
            .device_suspend(&dev_id, DmOptions::default().set_flags(DmFlags::DM_SUSPEND))?;
        self.dm_context.table_load(
            &dev_id,
            &[(
                0,
                self.size,
                "linear".to_string(),
                format!("{} 0", self.backing_device.display()),
            )],
            DmOptions::default(),
        )?;
        self.dm_context
            .device_suspend(&dev_id, DmOptions::default())?;

        Ok(())
    }
}

impl Drop for FailDevice {
    fn drop(&mut self) {
        fn drop_fail(dev: &mut FailDevice) -> StratisResult<()> {
            let dev_id = DevId::Name(DmName::new(dev.test_device_name.as_str())?);

            let (dev_info, _) = dev.dm_context.table_status(&dev_id, DmOptions::default())?;
            if dev_info.flags() & DmFlags::DM_SUSPEND == DmFlags::DM_SUSPEND {
                dev.dm_context
                    .device_suspend(&dev_id, DmOptions::default())?;
            }
            dev.dm_context
                .device_remove(&dev_id, DmOptions::default())?;

            Ok(())
        }

        if let Err(e) = drop_fail(self) {
            warn!(
                "Teardown of test device /dev/mapper/{} failed: {}",
                self.test_device_name, e
            );
        }
    }
}
