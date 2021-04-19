// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Blocking lock acquisition of a Tokio synchronization lock.
#[cfg(feature = "dbus_enabled")]
macro_rules! engine_lock {
    ($rwlock:expr, $method:ident) => {
        futures::executor::block_on($rwlock.$method())
    };
}
