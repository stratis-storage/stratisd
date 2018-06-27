// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Get ability to instantiate a devicemapper context.

use std::path::PathBuf;
use std::sync::{Once, ONCE_INIT};

use libudev::{self, Context};

use devicemapper::Device;

use stratis::{ErrorEnum, StratisError, StratisResult};

static INIT: Once = ONCE_INIT;
static mut UDEV_CONTEXT: Option<libudev::Result<Context>> = None;

pub fn get_udev_init() -> StratisResult<&'static Context> {
    unsafe {
        INIT.call_once(|| UDEV_CONTEXT = Some(Context::new()));
        match UDEV_CONTEXT {
            Some(Ok(ref context)) => Ok(context),
            // Can not move the error out of UDEV_CONTEXT, so synthesize a new
            // error.
            Some(Err(_)) => Err(StratisError::Engine(
                ErrorEnum::Error,
                "Failed to initialize udev context".into(),
            )),
            _ => panic!("UDEV_CONTEXT.is_some()"),
        }
    }
}

pub fn get_udev() -> &'static Context {
    get_udev_init().expect(
        "stratisd has already invoked get_udev_init() and exited if get_dm_init() returned an error",
    )
}

/// Get a devicemapper device and a device node from a libudev device.
/// Returns None if either could not be found.
pub fn get_device_devnode(device: &libudev::Device) -> Option<(Device, PathBuf)> {
    device.devnode().and_then(|devnode| {
        device
            .devnum()
            .and_then(|devnum| Some((Device::from(devnum), PathBuf::from(devnode))))
    })
}
