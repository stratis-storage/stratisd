// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(non_upper_case_globals)]
#[allow(non_snake_case)]
pub mod IEC {
    pub const Ki: u64 = 1024;
    pub const Mi: u64 = 1024 * Ki;
    pub const Gi: u64 = 1024 * Mi;
    pub const Ti: u64 = 1024 * Gi;
    pub const Pi: u64 = 1024 * Ti;
    pub const Ei: u64 = 1024 * Pi;
    // Ei is the maximum IEC unit expressible in u64.
}
