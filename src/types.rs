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

// macros for unsigned operations on Sectors and Bytes
macro_rules! unsigned_div {
    ($t: ty, $T: ident) => {
        impl Div<$t> for $T {
            type Output = $T;
            fn div(self, rhs: $t) -> $T {
                $T(self.0 / rhs as u64)
            }
        }
    }
}

macro_rules! unsigned_mul {
    ($t: ty, $T: ident) => {
        impl Mul<$t> for $T {
            type Output = $T;
            fn mul(self, rhs: $t) -> $T {
                $T(self.0 * rhs as u64)
            }
        }

        impl Mul<$T> for $t {
            type Output = $T;
            fn mul(self, rhs: $T) -> $T {
                $T(self as u64 * rhs.0)
            }
        }
    }
}

macro_rules! unsigned_rem {
    ($t: ty, $T: ident) => {
        impl Rem<$t> for $T {
            type Output = $T;
            fn rem(self, rhs: $t) -> $T {
                $T(self.0 % rhs as u64)
            }
        }
    }
}

custom_derive! {
    #[derive(NewtypeAdd, NewtypeAddAssign,
             NewtypeDeref,
             NewtypeFrom,
             NewtypeSub,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct Bytes(pub u64);
}

impl Bytes {
    /// Return the number of Sectors fully contained in these bytes.
    pub fn sectors(self) -> Sectors {
        Sectors(self.0 / SECTOR_SIZE as u64)
    }
}

unsigned_mul!(u64, Bytes);
unsigned_mul!(u32, Bytes);
unsigned_mul!(u16, Bytes);
unsigned_mul!(u8, Bytes);
unsigned_mul!(usize, Bytes);

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
    #[derive(NewtypeAdd, NewtypeAddAssign,
             NewtypeDeref,
             NewtypeFrom,
             NewtypeSub,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct Sectors(pub u64);
}

impl Sectors {
    /// The number of bytes in these sectors.
    pub fn bytes(&self) -> Bytes {
        Bytes(self.0 * SECTOR_SIZE as u64)
    }
}

unsigned_div!(u64, Sectors);
unsigned_div!(u32, Sectors);
unsigned_div!(u16, Sectors);
unsigned_div!(u8, Sectors);
unsigned_div!(usize, Sectors);

unsigned_mul!(u64, Sectors);
unsigned_mul!(u32, Sectors);
unsigned_mul!(u16, Sectors);
unsigned_mul!(u8, Sectors);
unsigned_mul!(usize, Sectors);

unsigned_rem!(u64, Sectors);
unsigned_rem!(u32, Sectors);
unsigned_rem!(u16, Sectors);
unsigned_rem!(u8, Sectors);
unsigned_rem!(usize, Sectors);

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
