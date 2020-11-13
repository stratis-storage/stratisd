// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{
    jsonrpc::consts::{OP_ERR, OP_OK, OP_OK_STR},
    stratis::StratisResult,
};

#[macro_export]
macro_rules! default_handler {
    ($respond:expr, $fn:path, $engine:expr, $default_value:expr $(, $args:expr)*) => {
        $respond.ok($crate::jsonrpc::server::utils::stratis_result_to_return(
            $fn(
                $engine,
                $($args),*
            ).await,
            $default_value,
        )).await
    }
}

pub fn stratis_result_to_return<T>(result: StratisResult<T>, default_value: T) -> (T, u16, String) {
    match result {
        Ok(r) => (r, OP_OK, OP_OK_STR.to_string()),
        Err(e) => (default_value, OP_ERR, e.to_string()),
    }
}
