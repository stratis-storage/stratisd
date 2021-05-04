// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// Blocking lock acquisition of a Tokio synchronization lock.
macro_rules! lock {
    ($rwlock:expr, $method:ident) => {
        futures::executor::block_on($rwlock.$method())
    };
}

/// Spawn blocking in an async context for a method that returns a result.
macro_rules! spawn_blocking {
    ($op:expr) => {
        tokio::task::spawn_blocking(move || $op)
            .await
            .map_err($crate::stratis::StratisError::from)
            .and_then(|res| res)
    };
}

#[cfg(test)]
macro_rules! block_on {
    ($op:expr) => {
        tokio::runtime::Runtime::new().unwrap().block_on($op)
    };
}
