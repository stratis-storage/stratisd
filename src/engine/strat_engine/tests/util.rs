// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use libmount;
use nix::mount::{MntFlags, umount2};

use devicemapper::{DevId, DmOptions};

use super::super::cmd;
use super::super::dm::{get_dm, get_dm_init};

#[allow(renamed_and_removed_lints)]
mod cleanup_errors {
    use libmount;
    use nix;
    use std;

    error_chain!{
        foreign_links {
            Ioe(std::io::Error);
            Mnt(libmount::mountinfo::ParseError);
            Nix(nix::Error);
        }
    }
}

use self::cleanup_errors::{Error, Result};

/// Attempt to remove all device mapper devices which match the stratis naming convention.
/// FIXME: Current implementation complicated by https://bugzilla.redhat.com/show_bug.cgi?id=1506287
fn dm_stratis_devices_remove() -> Result<()> {
    /// One iteration of removing devicemapper devices
    fn one_iteration() -> Result<(bool, Vec<String>)> {
        let mut progress_made = false;
        let mut remain = Vec::new();

        for n in get_dm()
            .list_devices()
            .map_err(|e| {
                let err_msg = "failed while listing DM devices, giving up";
                Error::with_chain(e, err_msg)
            })?
            .iter()
            .map(|d| &d.0)
            .filter(|n| n.to_string().starts_with("stratis-1"))
        {
            match get_dm().device_remove(&DevId::Name(n), &DmOptions::new()) {
                Ok(_) => progress_made = true,
                Err(_) => remain.push(n.to_string()),
            }
        }
        Ok((progress_made, remain))
    }

    /// Do one iteration of removals until progress stops. Return remaining
    /// dm devices.
    fn do_while_progress() -> Result<Vec<String>> {
        let mut result = one_iteration()?;
        while result.0 {
            result = one_iteration()?;
        }
        Ok(result.1)
    }

    || -> Result<()> {
        cmd::udev_settle().unwrap();
        get_dm_init().map_err(|err| Error::with_chain(err, "Unable to initialize DM"))?;
        do_while_progress().and_then(|remain| {
            if !remain.is_empty() {
                Err(format!("Some Stratis DM devices remaining: {:?}", remain).into())
            } else {
                Ok(())
            }
        })
    }().map_err(|e| e.chain_err(|| "Failed to ensure removal of all Stratis DM devices"))
}

/// Try and un-mount any filesystems that have the name stratis in the mount point, returning
/// immediately on the first one we are unable to unmount.
fn stratis_filesystems_unmount() -> Result<()> {
    || -> Result<()> {
        let mut mount_data = String::new();
        File::open("/proc/self/mountinfo")?.read_to_string(&mut mount_data)?;
        let parser = libmount::mountinfo::Parser::new(mount_data.as_bytes());

        for mount_point in parser
            .into_iter()
            .filter_map(|x| x.ok())
            .filter_map(|m| m.mount_point.into_owned().into_string().ok())
            .filter(|mp| mp.contains("stratis"))
        {
            umount2(&PathBuf::from(mount_point), MntFlags::MNT_DETACH)?;
        }

        Ok(())
    }().map_err(|e| e.chain_err(|| "Failed to ensure all Stratis filesystems were unmounted"))
}

/// When a unit test panics we can leave the system in an inconsistent state.  This function
/// tries to clean up by un-mounting any mounted file systems which contain the string
/// "stratis_testing" and then it tries to remove any device mapper tables which are also stratis
/// created.
pub fn clean_up() -> Result<()> {
    stratis_filesystems_unmount()?;
    dm_stratis_devices_remove()?;
    Ok(())
}
