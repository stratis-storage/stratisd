// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nix::libc::alarm as libc_alarm;

/// Ask for a SIGARLM to be sent to the process after some number of seconds.
/// See `man 2 alarm` for more.
// This should go in Nix crate.
pub fn alarm(seconds: u32) -> u32 {
    unsafe { libc_alarm(seconds) }
}
