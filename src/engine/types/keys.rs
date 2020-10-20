// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    convert::TryFrom,
    fmt::{self, Debug},
    hash::Hash,
};

use libcryptsetup_rs::SafeMemHandle;

use crate::stratis::{ErrorEnum, StratisError, StratisResult};

/// A handle for memory designed to safely handle Stratis passphrases. It can
/// be coerced to a slice reference for use in read-only operations.
pub struct SizedKeyMemory {
    mem: SafeMemHandle,
    size: usize,
}

impl SizedKeyMemory {
    pub fn new(mem: SafeMemHandle, size: usize) -> SizedKeyMemory {
        SizedKeyMemory { mem, size }
    }
}

impl Debug for SizedKeyMemory {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.as_ref())
    }
}

impl AsRef<[u8]> for SizedKeyMemory {
    fn as_ref(&self) -> &[u8] {
        &self.mem.as_ref()[..self.size]
    }
}

/// A data type respresenting a key description for the kernel keyring
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyDescription(String);

impl KeyDescription {
    /// Return the application-level key description (the key description with no
    /// Stratis prefix added).
    pub fn as_application_str(&self) -> &str {
        &self.0
    }
}

// Key descriptions with ';'s are disallowed because a key description
// containing a ';' will not be able to be correctly parsed from the kernel's
// describe string, which uses ';'s as field delimiters.
impl TryFrom<String> for KeyDescription {
    type Error = StratisError;

    fn try_from(s: String) -> StratisResult<KeyDescription> {
        if s.contains(';') {
            Err(StratisError::Engine(
                ErrorEnum::Invalid,
                format!("Key description {} contains a ';'", s),
            ))
        } else {
            Ok(KeyDescription(s))
        }
    }
}

impl<'a> TryFrom<&'a String> for KeyDescription {
    type Error = StratisError;

    fn try_from(s: &String) -> StratisResult<KeyDescription> {
        KeyDescription::try_from(s.to_owned())
    }
}
