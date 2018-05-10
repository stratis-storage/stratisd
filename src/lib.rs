#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]
#![cfg_attr(not(feature = "clippy"), allow(unknown_lints))]

#![allow(doc_markdown)]

extern crate devicemapper;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate byteorder;
extern crate uuid;
extern crate chrono;

#[cfg(feature="dbus_enabled")]
extern crate dbus;

extern crate tempfile;
extern crate rand;
extern crate serde;
extern crate mnt;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate log;
extern crate libudev;

#[cfg(test)]
#[macro_use]
extern crate error_chain;

#[cfg(test)]
extern crate quickcheck;

pub mod engine;

#[cfg(feature="dbus_enabled")]
pub mod dbus_api;

pub mod stratis;


#[macro_use]
extern crate macro_attr;
