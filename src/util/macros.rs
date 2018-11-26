/// Try Log Location -> tll
/// Used the same as try!, but also uses error! to log the file, line and error to allow us to
/// know where the error occurred.
#[macro_export]
macro_rules! tll {
    ($expr:expr) => {
        match $expr {
            ::std::result::Result::Ok(val) => val,
            ::std::result::Result::Err(err) => {
                error!("{} {} reason: {:?}", file!(), line!(), err);
                return ::std::result::Result::Err(::std::convert::From::from(err));
            }
        }
    };
    ($expr:expr,) => {
        tll!($expr)
    };
}
