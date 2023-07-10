// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// NOTE: cargo does not support mutually exclusive features. Because of this,
// our clippy Makefile target will enable both the dbus_enabled and min features.
// Currently, this module defaults to using D-Bus if both features are enabled.
// Both modules are declared if both features are active for the purpose of
// clippy.

#[cfg(feature = "dbus_enabled")]
mod dbus_support;
#[cfg(not(any(feature = "dbus_enabled", feature = "min")))]
mod dummy;
#[cfg(feature = "min")]
#[cfg_attr(feature = "dbus_enabled", allow(dead_code))]
mod jsonrpc_support;

#[cfg(feature = "dbus_enabled")]
pub use dbus_support::setup;

#[cfg(not(any(feature = "dbus_enabled", feature = "min")))]
pub use dummy::setup;

#[cfg(all(feature = "min", not(feature = "dbus_enabled")))]
pub use jsonrpc_support::setup;
