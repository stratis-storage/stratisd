#[cfg(test)]
macro_rules! test_async {
    ($expr:expr) => {
        tokio::runtime::Runtime::new().unwrap().block_on($expr)
    };
}

macro_rules! spawn_blocking {
    ($expr:expr) => {
        tokio::task::spawn_blocking(move || $expr)
            .await
            .map_err($crate::stratis::StratisError::from)
    };
}
