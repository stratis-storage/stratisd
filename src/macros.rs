// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(test)]
macro_rules! test_async {
    ($expr:expr) => {
        tokio::task::LocalSet::new().block_on(
            &tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            $expr,
        )
    };
}

#[cfg(feature = "engine")]
macro_rules! spawn_blocking {
    ($expr:expr) => {
        tokio::task::spawn_blocking(move || $expr)
            .await
            .map_err($crate::stratis::StratisError::from)
    };
}
