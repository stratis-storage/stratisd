#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]
#![cfg_attr(not(feature = "clippy"), allow(unknown_lints))]

#![allow(doc_markdown)]

extern crate devicemapper;
extern crate clap;
#[macro_use]
extern crate nix;
extern crate crc;
extern crate byteorder;
extern crate uuid;
extern crate chrono;
extern crate dbus;
extern crate term;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate log;

#[cfg(test)]
extern crate quickcheck;

pub mod engine;
pub mod dbus_api;
pub mod stratis;


#[macro_use]
extern crate custom_derive;
#[macro_use]
extern crate enum_derive;
