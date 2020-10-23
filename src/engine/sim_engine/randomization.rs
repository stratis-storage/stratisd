// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use rand::{rngs::OsRng, Rng};

#[derive(Debug)]
pub struct Randomizer {
    rng: OsRng,
    denominator: u32,
}

impl Default for Randomizer {
    fn default() -> Randomizer {
        Randomizer {
            rng: OsRng,
            denominator: 0u32,
        }
    }
}

impl Randomizer {
    /// Throw a denominator sided die, returning true if 1 comes up
    /// If denominator is 0, return false
    pub fn throw_die(&mut self) -> bool {
        if self.denominator == 0 {
            false
        } else {
            self.rng.gen_ratio(1, self.denominator)
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
