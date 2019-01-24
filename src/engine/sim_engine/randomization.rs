// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use rand::{OsRng, Rng};

pub struct Randomizer {
    rng: OsRng,
    denominator: u32,
}

impl Default for Randomizer {
    fn default() -> Randomizer {
        Randomizer {
            rng: OsRng::new().expect("rng init shouldn't fail"),
            denominator: 0u32,
        }
    }
}

/// Implement Debug explicitly as ThreadRng does not derive it.
/// See: https://github.com/rust-lang-nursery/rand/issues/118
impl fmt::Debug for Randomizer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{{Randomizer {:?}", self.denominator)
    }
}

impl Randomizer {
    /// Throw a denominator sided die, returning true if 1 comes up
    /// If denominator is 0, return false
    pub fn throw_die(&mut self) -> bool {
        if self.denominator == 0 {
            false
        } else {
            self.rng.gen_weighted_bool(self.denominator)
        }
    }

    /// Set the probability of a failure.
    pub fn set_probability(&mut self, denominator: u32) -> &mut Self {
        self.denominator = denominator;
        self
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::any;

    use super::*;

    proptest! {
        #[test]
        /// Verify that if the denominator is 0 the result is always false,
        /// if 1, always true.
        fn denominator_result(denominator in any::<u32>()) {
            let result = Randomizer::default()
                .set_probability(denominator)
                .throw_die();
            prop_assert!(denominator > 1
                         || (denominator != 0 && !result)
                         || (denominator == 0 && result));
        }
    }
}
