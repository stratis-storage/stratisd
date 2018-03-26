// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Utilities to support Stratis.

use std::{thread, time};
use std::path::Path;

use blkid;
use blkid_sys;
use libudev;

use super::super::super::errors::{EngineError, EngineResult, ErrorEnum};

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> EngineResult<Option<String>> {
    #![allow(let_and_return)]
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("DEVTYPE", "disk")?;

    let result = enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| dev_node_search == d))
        .map_or(Ok(None), |dev| {
            dev.property_value("ID_WWN")
                .map_or(Ok(None), |i| {
                    i.to_str()
                        .ok_or_else(|| {
                                        EngineError::Engine(ErrorEnum::Error,
                                                            format!("Unable to convert {:?} to str",
                                                                    i))
                                    })
                        .map(|i| Some(String::from(i)))
                })
        });

    result
}

// Probes a block device using the libblkid library and returns Some(None) if the device is empty or
// Some(String) with the String having similar content as the output of `blkid -p <device>.
pub fn device_signature(path: &Path) -> Result<Option<String>, blkid::BlkidError> {

    let d = blkid::BlkId::new(path)?;

    d.enable_partitions()?
        .enable_superblocks()?
        .set_superblock_flags(blkid_sys::BLKID_SUBLKS_USAGE | blkid_sys::BLKID_SUBLKS_TYPE |
                              blkid_sys::BLKID_SUBLKS_BADCSUM |
                              blkid_sys::BLKID_SUBLKS_UUID)?;

    // There is potential for the probe to fail if the device is in use/locked, lets loop a bit
    // to ride out these potentially temporary errors.
    let mut num_retries = 5;
    let probe_result = loop {
        match d.do_safe_probe() {
            Ok(p) => break p,
            Err(e) => {
                num_retries -= 1;
                if num_retries == 0 {
                    return Err(e);
                }
                thread::sleep(time::Duration::from_millis(100));
            }
        }
    };

    Ok(match probe_result {
           1 => None,
           -2 => Some(String::from("Ambiguous device usage detected")),
           _ => {
               Some(d.get_values_map()?
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<String>>()
                        .join(" "))
           }
       })
}
