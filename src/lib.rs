#![allow(clippy::doc_markdown)]

extern crate devicemapper;
#[macro_use]
extern crate nix;
extern crate byteorder;
extern crate chrono;
extern crate crc;
extern crate uuid;

extern crate libc;
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

#[cfg(test)]
#[macro_use]
extern crate error_chain;

extern crate failure;
extern crate varlink;

//#[macro_use]
//extern crate failure_derive;

#[cfg(test)]
#[macro_use]
extern crate proptest;

#[cfg(test)]
#[macro_use]
extern crate matches;

#[macro_use]
extern crate lazy_static;

pub mod engine;

extern crate chainerror;

pub mod stratis;
pub mod varlink_api;

#[macro_use]
extern crate macro_attr;
