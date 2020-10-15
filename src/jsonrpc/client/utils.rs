// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::os::unix::io::RawFd;

use nix::sys::{
    socket::{sendmsg, ControlMessage, MsgFlags},
    uio::IoVec,
};

#[macro_export]
macro_rules! do_request {
    ($fn:path $(, $args:expr)*) => {{
        match async_std::task::block_on(async {
            let transport = jsonrpsee::transport::http::HttpTransportClient::new($crate::jsonrpc::consts::RPC_CONNADDR);
            let mut client = jsonrpsee::raw::RawClient::new(transport);
            $fn(&mut client $(, $args)*).await
        }) {
            Ok(r) => r,
            Err(e) => return Err(
                $crate::stratis::StratisError::Error(format!("Transport error: {}", e))
            ),
        }
    }}
}

#[macro_export]
macro_rules! do_request_standard {
    ($fn:path $(, $args:expr)*) => {{
        let (changed, rc, rs) = $crate::do_request!($fn $(, $args)*);
        if rc != 0 {
            Err(StratisError::Error(rs))
        } else if !changed {
            Err(StratisError::Error(
                "The requested action had no effect".to_string(),
            ))
        } else {
            Ok(())
        }
    }}
}

#[macro_export]
macro_rules! left_align {
    ($string:expr, $max_length:expr) => {{
        let len = $string.len();
        $string + vec![" "; $max_length - len + 3].join("").as_str()
    }};
}

#[macro_export]
macro_rules! right_align {
    ($string:expr, $max_length:expr) => {
        vec![" "; $max_length - $string.len() + 3].join("") + $string.as_str()
    };
}

#[macro_export]
macro_rules! align {
    ($string:expr, $max_length:expr, $align:tt) => {
        if $align == ">" {
            $crate::right_align!($string, $max_length)
        } else {
            $crate::left_align!($string, $max_length)
        }
    };
}

#[macro_export]
macro_rules! print_table {
    ($($heading:expr, $values:expr, $align:tt);*) => {{
        let (lengths_same, lengths) = vec![$($values.len()),*]
            .into_iter()
            .fold((true, None), |(is_same, len_opt), len| {
                if len_opt.is_none() {
                    (true, Some(len))
                } else {
                    (is_same && len_opt == Some(len), len_opt)
                }
            });
        if !lengths_same {
            return Err($crate::stratis::StratisError::Error(
                "All values parameters must be the same length".to_string()
            ));
        }
        let mut output = vec![String::new(); lengths.unwrap_or(0) + 1];
        $(
            let max_length = $values
                .iter()
                .fold($heading.len(), |acc, val| {
                    if val.len() > acc {
                        val.len()
                    } else {
                        acc
                    }
                });
            if let Some(string) = output.get_mut(0) {
                string.push_str($crate::align!($heading.to_string(), max_length, $align).as_str());
            }
            for (index, row_seg) in $values.into_iter()
                .map(|s| $crate::align!(s, max_length, $align))
                .enumerate()
            {
                if let Some(string) = output.get_mut(index + 1) {
                    string.push_str(row_seg.as_str());
                }
            }
        )*
        for row in output.into_iter() {
            println!("{}", row);
        }
    }};
}

pub fn send_fd_to_sock(unix_fd: RawFd, fd: RawFd) -> Result<(), nix::Error> {
    sendmsg(
        unix_fd,
        &[IoVec::from_slice(&[0, 0, 0, 0])],
        &[ControlMessage::ScmRights(&[fd])],
        MsgFlags::empty(),
        None,
    )?;
    Ok(())
}
