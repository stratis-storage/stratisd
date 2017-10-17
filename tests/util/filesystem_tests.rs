// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Test the functionality of stratis filesystems.
extern crate devicemapper;
extern crate uuid;
extern crate env_logger;
extern crate nix;
extern crate tempdir;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use self::nix::mount::{MsFlags, mount, umount};

use self::tempdir::TempDir;

use self::devicemapper::{Bytes, DM, IEC, SECTOR_SIZE};

use libstratis::engine::Pool;
use libstratis::engine::Filesystem;
use libstratis::engine::strat_engine::filesystem::{FILESYSTEM_LOWATER, fs_usage};
use libstratis::engine::strat_engine::pool::StratPool;
use libstratis::engine::types::Redundancy;

/// Verify that the logical space allocated to a filesystem is expanded when
/// the number of sectors written to the filesystem causes the free space to
/// dip below the FILESYSTEM_LOWATER mark. Verify that the space has been
/// expanded by calling filesystem.check() then looking at the total space
/// compared to the original size.
pub fn test_xfs_expand(paths: &[&Path]) -> () {
    let dm = DM::new().unwrap();
    // Create a filesytem as small as possible.  Allocate 1 MiB bigger than
    // the low water mark.
    let fs_size = FILESYSTEM_LOWATER + Bytes(IEC::Mi).sectors();

    let (mut pool, _) =
        StratPool::initialize("stratis_test_pool", &dm, paths, Redundancy::NONE, true).unwrap();
    let &(_, fs_uuid) = pool.create_filesystems(&[("stratis_test_filesystem", Some(fs_size))])
        .unwrap()
        .first()
        .unwrap();
    // Braces to ensure f is closed before destroy and the borrow of pool is complete
    {
        let filesystem = pool.get_mut_strat_filesystem(fs_uuid).unwrap();
        // Write 2 MiB of data. The filesystem's free space is now 1 MiB below
        // FILESYSTEM_LOWATER.
        let write_size = Bytes(IEC::Mi * 2).sectors();
        let tmp_dir = TempDir::new("stratis_testing").unwrap();
        mount(Some(&filesystem.devnode()),
              tmp_dir.path(),
              Some("xfs"),
              MsFlags::empty(),
              None as Option<&str>)
                .unwrap();
        let buf = &[1u8; SECTOR_SIZE];
        for i in 0..*write_size {
            let file_path = tmp_dir.path().join(format!("stratis_test{}.txt", i));
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .open(file_path)
                .unwrap();
            if f.write_all(buf).is_err() {
                break;
            }
        }
        let (orig_fs_total_bytes, _) = fs_usage(&tmp_dir.path()).unwrap();
        // Simulate handling a DM event by running a filesystem check.
        filesystem.check(&dm).unwrap();
        let (fs_total_bytes, _) = fs_usage(&tmp_dir.path()).unwrap();
        assert!(fs_total_bytes > orig_fs_total_bytes);
        umount(tmp_dir.path()).unwrap();
    }
    pool.destroy_filesystems(&[fs_uuid]).unwrap();
    pool.teardown().unwrap();
}
