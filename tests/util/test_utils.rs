// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! assert_ok {
    ($e:expr) => (match $e {
        Ok(val) => val,
        Err(err) => {
            error!("FAILED : {:?}", err);
            assert!(false);
            return;
        }
    });
}

macro_rules! try_log_error {
    ($e:expr) => (match $e {
        Ok(val) => info!("{:?}", val),
        Err(err) =>
            error!("FAILED : {:?}", err),
    });
}
