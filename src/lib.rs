#[cfg(feature = "min")]
#[cfg_attr(feature = "dbus_enabled", allow(dead_code))]
#[macro_use]
extern crate nix;

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate log;

#[cfg(test)]
#[macro_use]
extern crate proptest;

#[cfg(test)]
#[macro_use]
extern crate matches;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate serde_json;

#[macro_use]
extern crate libcryptsetup_rs;

#[macro_use]
mod macros;

pub mod engine;

#[cfg(feature = "dbus_enabled")]
pub mod dbus_api;

pub mod stratis;

#[cfg(feature = "min")]
pub mod jsonrpc;

#[cfg(feature = "systemd_compat")]
pub mod systemd;
