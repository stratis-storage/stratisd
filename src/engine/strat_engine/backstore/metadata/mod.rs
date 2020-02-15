// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Transform a constant in sectors to a constant in bytes
#[macro_export]
macro_rules! bytes {
    ($number:expr) => {
        $number * devicemapper::SECTOR_SIZE
    };
}

mod bda;
mod mda;
mod sizes;
mod static_header;

pub use self::{
    bda::BDA,
    sizes::{BDAExtendedSize, BlockdevSize, MDADataSize},
    static_header::{device_identifiers, disown_device, StratisIdentifiers},
};
