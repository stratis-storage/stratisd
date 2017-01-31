// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::fmt;
use std::error::Error;
use std::borrow::Cow;
use std::fmt::Display;
use std::ops::{Div, Mul, Rem};

use nix;
use term;
use dbus;
use serde;

use consts::SECTOR_SIZE;

pub type StratisResult<T> = Result<T, StratisError>;

custom_derive! {
    #[derive(NewtypeFrom, NewtypeAdd, NewtypeSub, NewtypeDeref,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct Bytes(pub u64);
}

impl Bytes {
    /// Return the number of Sectors fully contained in these bytes.
    pub fn sectors(self) -> Sectors {
        Sectors(self.0 / SECTOR_SIZE as u64)
    }
}

impl Mul<usize> for Bytes {
    type Output = Bytes;
    fn mul(self, rhs: usize) -> Bytes {
        Bytes(self.0 * rhs as u64)
    }
}

impl Mul<u64> for Bytes {
    type Output = Bytes;
    fn mul(self, rhs: u64) -> Bytes {
        Bytes(self.0 * rhs)
    }
}


impl Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{} bytes", self.0)
    }
}

// Use distinct 'newtype' types for sectors and sector offsets for type safety.
// When needed, these can still be derefed to u64.
// Derive a bunch of stuff so we can do ops on them.
//
custom_derive! {
    #[derive(NewtypeFrom, NewtypeAdd, NewtypeSub, NewtypeDeref,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct Sectors(pub u64);
}

impl Sectors {
    /// The number of bytes in these sectors.
    pub fn bytes(&self) -> Bytes {
        Bytes(self.0 * SECTOR_SIZE as u64)
    }
}

impl Div<usize> for Sectors {
    type Output = Sectors;
    fn div(self, rhs: usize) -> Sectors {
        Sectors(self.0 / rhs as u64)
    }
}

impl Div<u64> for Sectors {
    type Output = Sectors;
    fn div(self, rhs: u64) -> Sectors {
        Sectors(self.0 / rhs)
    }
}

impl Mul<usize> for Sectors {
    type Output = Sectors;
    fn mul(self, rhs: usize) -> Sectors {
        Sectors(self.0 * rhs as u64)
    }
}

impl Mul<u64> for Sectors {
    type Output = Sectors;
    fn mul(self, rhs: u64) -> Sectors {
        Sectors(self.0 * rhs)
    }
}

impl Rem<usize> for Sectors {
    type Output = Sectors;
    fn rem(self, rhs: usize) -> Sectors {
        Sectors(self.0 % rhs as u64)
    }
}

impl Rem<u64> for Sectors {
    type Output = Sectors;
    fn rem(self, rhs: u64) -> Sectors {
        Sectors(self.0 % rhs)
    }
}

impl Display for Sectors {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{} sectors", self.0)
    }
}

impl serde::Serialize for Sectors {
    fn serialize<S>(&self, serializer: &mut S) -> Result<(), S::Error>
        where S: serde::Serializer
    {
        serializer.serialize_u64(**self)
    }
}

impl serde::Deserialize for Sectors {
    fn deserialize<D>(deserializer: &mut D) -> Result<Sectors, D::Error>
        where D: serde::de::Deserializer
    {
        let val = try!(serde::Deserialize::deserialize(deserializer));
        Ok(Sectors(val))
    }
}


// An error type for errors generated within Stratis
//
#[derive(Debug)]
pub struct InternalError(pub Cow<'static, str>);

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0)
    }
}

impl Error for InternalError {
    fn description(&self) -> &str {
        &self.0
    }
}

// Define a common error enum.
// See http://blog.burntsushi.net/rust-error-handling/
#[derive(Debug)]
pub enum StratisError {
    Stratis(InternalError),
    Io(io::Error),
    Nix(nix::Error),
    Dbus(dbus::Error),
    Term(term::Error),
}

impl fmt::Display for StratisError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            StratisError::Stratis(ref err) => write!(f, "Stratis error: {}", err.0),
            StratisError::Io(ref err) => write!(f, "IO error: {}", err),
            StratisError::Nix(ref err) => write!(f, "Nix error: {}", err.errno().desc()),
            StratisError::Dbus(ref err) => {
                write!(f, "Dbus error: {}", err.message().unwrap_or("Unknown"))
            }
            StratisError::Term(ref err) => write!(f, "Term error: {}", err),
        }
    }
}

impl Error for StratisError {
    fn description(&self) -> &str {
        match *self {
            StratisError::Stratis(ref err) => &err.0,
            StratisError::Io(ref err) => err.description(),
            StratisError::Nix(ref err) => err.errno().desc(),
            StratisError::Dbus(ref err) => err.message().unwrap_or("D-Bus Error"),
            StratisError::Term(ref err) => Error::description(err),
        }
    }

    fn cause(&self) -> Option<&Error> {
        match *self {
            StratisError::Stratis(ref err) => Some(err),
            StratisError::Io(ref err) => Some(err),
            StratisError::Nix(ref err) => Some(err),
            StratisError::Dbus(ref err) => Some(err),
            StratisError::Term(ref err) => Some(err),
        }
    }
}

impl From<InternalError> for StratisError {
    fn from(err: InternalError) -> StratisError {
        StratisError::Stratis(err)
    }
}

impl From<io::Error> for StratisError {
    fn from(err: io::Error) -> StratisError {
        StratisError::Io(err)
    }
}



impl From<nix::Error> for StratisError {
    fn from(err: nix::Error) -> StratisError {
        StratisError::Nix(err)
    }
}

impl From<dbus::Error> for StratisError {
    fn from(err: dbus::Error) -> StratisError {
        StratisError::Dbus(err)
    }
}

impl From<term::Error> for StratisError {
    fn from(err: term::Error) -> StratisError {
        StratisError::Term(err)
    }
}
