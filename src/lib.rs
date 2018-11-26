#![cfg_attr(not(feature = "clippy"), allow(unknown_lints))]
#![allow(doc_markdown)]

extern crate devicemapper;
#[macro_use]
extern crate nix;
extern crate byteorder;
extern crate chrono;
extern crate crc;
extern crate uuid;

#[cfg(feature = "dbus_enabled")]
extern crate dbus;

extern crate libmount;
extern crate rand;
extern crate serde;
extern crate tempfile;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate log;
extern crate libudev;
extern crate walkdir;

#[cfg(test)]
#[macro_use]
extern crate error_chain;

#[cfg(test)]
#[macro_use]
extern crate proptest;

#[cfg(test)]
#[macro_use]
extern crate matches;

#[macro_use]
extern crate lazy_static;

#[macro_use]
pub mod util;

pub mod engine;

#[cfg(feature = "dbus_enabled")]
pub mod dbus_api;

pub mod stratis;
