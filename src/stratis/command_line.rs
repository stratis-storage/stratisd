// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{builder::PossibleValue, ValueEnum};

use strum::VariantArray;

pub use crate::engine::UnlockMethod;

impl ValueEnum for UnlockMethod {
    fn value_variants<'a>() -> &'a [UnlockMethod] {
        UnlockMethod::VARIANTS
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        let value: &'static str = self.into();
        Some(PossibleValue::new(value))
    }
}
