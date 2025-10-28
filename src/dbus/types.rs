// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use zbus::zvariant::{Signature, Type};

use crate::engine::{KeyDescription, Name, PoolUuid};

impl Type for KeyDescription {
    const SIGNATURE: &Signature = &Signature::Str;
}

impl Type for PoolUuid {
    const SIGNATURE: &Signature = &Signature::Str;
}

impl Type for Name {
    const SIGNATURE: &Signature = &Signature::Str;
}

#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub enum DbusErrorEnum {
    OK = 0,
    ERROR = 1,
}
