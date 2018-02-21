// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use devicemapper::{DM, DevId, DmFlags, DmResult};

use mnt::get_submounts;
use nix::mount::{MntFlags, umount2};


mod cleanup_errors {
    use mnt;
    use nix;

    error_chain!{
        foreign_links {
            Mnt(mnt::ParseError);
            Nix(nix::Error);
        }
    }
}

use self::cleanup_errors::{Error, Result, ResultExt};


/// Attempt to remove all device mapper devices which match the stratis naming convention.
/// FIXME: Current implementation complicated by https://bugzilla.redhat.com/show_bug.cgi?id=1506287
fn dm_stratis_devices_remove() -> Result<()> {

    /// One iteration of removing devicemapper devices
    fn one_iteration(dm: &DM) -> DmResult<(bool, Vec<String>)> {
        let mut progress_made = false;
        let mut remain = Vec::new();

        for d in dm.list_devices()?
                .iter()
                .filter(|d| format!("{}", d.0.as_ref()).starts_with("stratis-1")) {

            match dm.device_remove(&DevId::Name(&d.0), DmFlags::empty()) {
                Ok(_) => progress_made = true,
                Err(_) => remain.push(format!("{}", d.0.as_ref())),
            }
        }
        Ok((progress_made, remain))
    }

    let dm = DM::new().chain_err(|| "Unable to initialize DM")?;

    loop {
        let (progress_made, remain) = one_iteration(&dm)
            .map_err(|e| {
                Error::with_chain(e,
                                  "Error while attempting to remove stratis device mapper devices")}
                )?;

        if !progress_made {
            if remain.len() != 0 {
                bail!("We were unable to remove all stratis device mapper devices {:?}",
                      remain);
            }
            break;
        }
    }

    Ok(())
}

/// Try and un-mount any filesystems that have the name stratis in the mount point, returning
/// immediately on the first one we are unable to unmount.
fn stratis_filesystems_unmount() -> Result<()> {
    || -> Result<()> {
        let mounts = get_submounts(&PathBuf::from("/"))?;
        for m in mounts
                .iter()
                .filter(|m| m.file.to_str().map_or(false, |s| s.contains("stratis"))) {
            umount2(&m.file, MntFlags::MNT_DETACH)?;
        }
        Ok(())
    }()
            .map_err(|e| Error::with_chain(e, "unable to unmount all stratis filesystems"))
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
