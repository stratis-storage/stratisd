// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use devicemapper::{DM, DmFlags, DevId};

use nix::mount::umount;
use mnt::get_submounts;

/// Attempt to remove all device mapper devices which match the stratis naming convention.
/// FIXME: Current implementation complicated by https://bugzilla.redhat.com/show_bug.cgi?id=1506287
fn dm_stratis_devices_remove() {

    let dm = DM::new().unwrap();

    loop {
        let mut progress_made = false;
        for d in dm.list_devices()
                .unwrap()
                .iter()
                .filter(|d| format!("{}", d.0.as_ref()).starts_with("stratis-1")) {
            progress_made |= dm.device_remove(&DevId::Name(&d.0), DmFlags::empty())
                .is_ok();
        }

        if !progress_made {
            break;
        }
    }
}

/// Try and un-mount any filesystems that have the name stratis in the mount point.
fn stratis_filesystems_unmount() {
    for m in get_submounts(&PathBuf::from("/"))
            .unwrap()
            .iter()
            .filter(|m| m.file.to_str().map_or(false, |s| s.contains("stratis"))) {
        umount(&m.file).unwrap();
    }
}

/// When a unit test panics we can leave the system in an inconsistent state.  This function
/// tries to clean up by un-mounting any mounted file systems which contain the string
/// "stratis_testing" and then it tries to remove any device mapper tables which are also stratis
/// created.
pub fn clean_up() -> () {
    stratis_filesystems_unmount();
    dm_stratis_devices_remove();
}
