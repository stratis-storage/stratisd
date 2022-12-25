#[cfg(test)]
macro_rules! test_async {
    ($expr:expr) => {
        tokio::task::LocalSet::new().block_on(
            &tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .unwrap(),
            $expr,
        )
    };
}

macro_rules! spawn_blocking {
    ($expr:expr) => {
        tokio::task::spawn_blocking(move || $expr)
            .await
            .map_err($crate::stratis::StratisError::from)
    };
}
