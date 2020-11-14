// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{
    jsonrpc::consts::{OP_ERR, OP_OK, OP_OK_STR},
    stratis::StratisResult,
};

macro_rules! expects_fd {
    ($fd_opt:expr, $ret:ident, $default:expr, true) => {
        match $fd_opt {
            Some(fd) => fd,
            None => {
                let res = Err($crate::stratis::StratisError::Error(
                    "Method expected a file descriptor and did not receive one".to_string(),
                ));
                return $crate::jsonrpc::interface::StratisRet::$ret(
                    $crate::jsonrpc::server::utils::stratis_result_to_return(res, $default),
                );
            }
        }
    };
    ($fd_opt:expr, $ret:ident, $default:expr, false) => {
        match $fd_opt {
            Some(fd) => {
                if let Err(e) = nix::unistd::close(fd) {
                    warn!(
                        "Failed to close file descriptor {}: {}; a file descriptor \
                        may have been leaked",
                        fd, e,
                    );
                }
                let res = Err($crate::stratis::StratisError::Error(
                    "Method did not expect a file descriptor and received one \
                    anyway; file descriptor has been closed"
                        .to_string(),
                ));
                return $crate::jsonrpc::interface::StratisRet::$ret(
                    $crate::jsonrpc::server::utils::stratis_result_to_return(res, $default),
                );
            }
            None => (),
        }
    };
}

pub fn stratis_result_to_return<T>(result: StratisResult<T>, default_value: T) -> (T, u16, String) {
    match result {
        Ok(r) => (r, OP_OK, OP_OK_STR.to_string()),
        Err(e) => (default_value, OP_ERR, e.to_string()),
    }
}
