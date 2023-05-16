// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(feature = "engine")]
#[macro_use]
extern crate nix;

#[cfg(feature = "engine")]
#[macro_use]
extern crate serde_derive;
#[cfg(feature = "engine")]
#[macro_use]
extern crate log;

#[cfg(feature = "engine")]
#[cfg(test)]
#[macro_use]
extern crate proptest;

#[cfg(feature = "engine")]
#[cfg(test)]
#[macro_use]
extern crate assert_matches;

#[cfg(feature = "engine")]
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "engine")]
#[macro_use]
extern crate serde_json;

#[cfg(feature = "engine")]
#[macro_use]
extern crate libcryptsetup_rs;

#[cfg(feature = "engine")]
#[macro_use]
mod macros;

#[cfg(feature = "engine")]
pub mod engine;

#[cfg(feature = "dbus_enabled")]
pub mod dbus_api;

#[cfg(feature = "engine")]
pub mod stratis;

#[cfg(feature = "min")]
pub mod jsonrpc;

#[cfg(feature = "systemd_compat")]
pub mod systemd;
