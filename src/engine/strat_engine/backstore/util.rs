// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Utilities to support Stratis.

use std::path::Path;

use libudev;

use stratis::{ErrorEnum, StratisError, StratisResult};

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    #![allow(let_and_return)]
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("DEVTYPE", "disk")?;

    let result = enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| dev_node_search == d))
        .map_or(Ok(None), |dev| {
            dev.property_value("ID_WWN").map_or(Ok(None), |i| {
                i.to_str()
                    .ok_or_else(|| {
                        StratisError::Engine(
                            ErrorEnum::Error,
                            format!("Unable to convert {:?} to str", i),
                        )
                    })
                    .map(|i| Some(String::from(i)))
            })
        });

    result
}
