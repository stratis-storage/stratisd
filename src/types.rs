// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io;
use std::fmt;
use std::error::Error;
use std::borrow::Cow;
use std::ops::Add;

use nix;
use term;
use dbus;

pub type StratisResult<T> = Result<T, StratisError>;

//
// Use distinct 'newtype' types for sectors and sector offsets for type safety.
// When needed, these can still be derefed to u64.
// Derive a bunch of stuff so we can do ops on them.
//
custom_derive! {
    #[derive(NewtypeFrom, NewtypeAdd, NewtypeSub, NewtypeDeref,
             NewtypeBitAnd, NewtypeNot, NewtypeDiv, NewtypeRem,
             NewtypeMul,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct Sectors(pub u64);
}

// `SumSectors` can be discarded once `std::iter::Sum` is stable.
pub trait SumSectors: Iterator
where Sectors: Add<Self::Item, Output=Sectors>
{
    fn sum_sectors(self) -> Sectors
        where Self: Sized
    {
        self.fold(Sectors(0), Add::add)
    }
}

impl<T: Iterator> SumSectors for T
where Sectors: Add<T::Item, Output=Sectors>
{
}


custom_derive! {
    #[derive(NewtypeFrom, NewtypeAdd, NewtypeSub, NewtypeDeref,
             NewtypeBitAnd, NewtypeNot, NewtypeDiv, NewtypeRem,
             NewtypeMul,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct SectorOffset(pub u64);
}



// A type for Data Blocks as used by the thin pool.
custom_derive! {
    #[derive(NewtypeFrom, NewtypeAdd, NewtypeSub, NewtypeDeref,
             NewtypeBitAnd, NewtypeNot, NewtypeDiv, NewtypeRem,
             NewtypeMul,
             Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
    pub struct DataBlocks(pub u64);
}


//
// An error type for errors generated within Stratis
//
#[derive(Debug, Clone)]
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
            StratisError::Dbus(ref err) => write!(
                f, "Dbus error: {}", err.message().unwrap_or("Unknown")),
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
