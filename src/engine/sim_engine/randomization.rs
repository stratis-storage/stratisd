// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::fmt;

use rand::{thread_rng, Rng, ThreadRng};

pub struct Randomizer {
    rng: ThreadRng,
    denominator: u32,
}

impl Default for Randomizer {
    fn default() -> Randomizer {
        Randomizer {
            rng: thread_rng(),
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
    use quickcheck::QuickCheck;

    use super::*;

    #[test]
    fn prop_denominator_result() {
        /// Verify that if the denominator is 0 the result is always false,
        /// if 1, always true.
        fn denominator_result(denominator: u32) -> bool {
            let result = Randomizer::default()
                .set_probability(denominator)
                .throw_die();
            if denominator > 1 {
                true
            } else {
                if denominator == 0 {
                    result == false
                } else {
                    result == true
                }
            }
        }
        QuickCheck::new()
            .tests(30)
            .quickcheck(denominator_result as fn(u32) -> bool);
    }
}
