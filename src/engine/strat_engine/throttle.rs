// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fs::OpenOptions;
use std::io::Write;

use devicemapper::{Bytes, Device};
use walkdir::WalkDir;

use stratis::StratisResult;

const CGROUP_PATH: &str = "/sys/fs/cgroup/blkio/";
const THROTTLE_BPS_PATH: &str = "blkio.throttle.write_bps_device";

/// Use block throttling to limit writes to a certain amount, or stop
/// throttling by passing `None` for `bytes_per_sec`.
// The underlying APIs here are... in flux. Hierarchical blkio throttling
// doesn't work on all configs. What we're doing for the moment is setting the
// throttle in *all* cgroups that are present.
pub fn set_write_throttling(device: Device, bytes_per_sec: Option<Bytes>) -> StratisResult<()> {
    // Setting to u64::max_value() removes throttling.
    let value = bytes_per_sec.unwrap_or_else(|| Bytes(u64::max_value()));

    for mut cg_entry in WalkDir::new(CGROUP_PATH)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str() == Some(THROTTLE_BPS_PATH))
    {
        OpenOptions::new()
            .write(true)
            .open(cg_entry.path())
            .and_then(|mut f| f.write_all(format!("{} {}", device, *value).as_bytes()))?;
    }

    match bytes_per_sec {
        None => info!("Throttling disabled for device {}", device),
        Some(amt) => info!("Throttled device {} to {} bytes/sec", device, *amt),
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    use std::fs::{self, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::str::FromStr;

    use chrono::{Duration, Utc};

    use devicemapper::{Device, Sectors, IEC};

    use stratis::StratisResult;

    use super::super::tests::{loopbacked, real};

    use super::super::device::wipe_sectors;

    fn get_write_throttled_devices() -> StratisResult<Vec<(Device, u64)>> {
        // Find all cgroup subdirectories
        let mut cg_dirs = fs::read_dir(CGROUP_PATH)?
            .into_iter()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();

        // If there are no subdirs, get the root cgroup
        if cg_dirs.is_empty() {
            cg_dirs.push(CGROUP_PATH.into());
        }

        let path = cg_dirs.first_mut().expect("there is at least one");
        path.push(THROTTLE_BPS_PATH);

        let mut f = OpenOptions::new().read(true).open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let buf = String::from_utf8_lossy(&buf);
        Ok(buf
            .lines()
            .map(|line| {
                let mut i = line.split_whitespace();
                let a = i.next().expect("kernel is never wrong");
                let device = Device::from_str(a).expect("kernel is never wrong");
                let b = i.next().expect("kernel is never wrong");
                (device, b.parse::<u64>().unwrap())
            })
            .collect())
    }

    fn test_write_throttling(paths: &[&Path]) {
        let path = paths[0];
        let f = OpenOptions::new().write(true).open(path).unwrap();
        let d: Device = f.metadata().unwrap().rdev().into();

        // Ensure a previous test run didn't leave any throttling
        set_write_throttling(d, None).unwrap();

        // See how fast unthrottled writes are
        let start = Utc::now();
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi * 50).sectors()).unwrap();
        let end = Utc::now();
        println!(
            "unthrottled write took {}",
            end.signed_duration_since(start)
        );

        // Limit writes to 1MiB/sec
        set_write_throttling(d, Some(Bytes(IEC::Mi))).unwrap();

        // Check that throttled writes are indeed slower
        let start = Utc::now();
        wipe_sectors(path, Sectors(0), Bytes(IEC::Mi * 50).sectors()).unwrap();
        let end = Utc::now();
        println!("throttled write took {}", end.signed_duration_since(start));
        assert!(end.signed_duration_since(start) > Duration::seconds(50));

        // Verify that setting to None for the device removes the throttling
        assert_eq!(get_write_throttled_devices().unwrap().len(), 1);
        set_write_throttling(d, None).unwrap();
        assert_eq!(get_write_throttled_devices().unwrap().len(), 0);
    }

    #[test]
    pub fn loop_test_write_throttling() {
        loopbacked::test_with_spec(
            loopbacked::DeviceLimits::Exactly(1, Some(Bytes(IEC::Mi * 50).sectors())),
            test_write_throttling,
        );
    }

    #[test]
    pub fn real_test_write_throttling() {
        real::test_with_spec(
            real::DeviceLimits::Exactly(1, Some(Bytes(IEC::Mi * 50).sectors()), None),
            test_write_throttling,
        );
    }

}
