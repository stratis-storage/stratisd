// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod device_info;
#[allow(clippy::module_inception)]
mod liminal;
mod setup;

pub use self::liminal::LiminalDevices;

#[cfg(test)]
pub use self::{
    device_info::LStratisInfo,
    setup::{get_bdas, get_blockdevs, get_metadata},
};
