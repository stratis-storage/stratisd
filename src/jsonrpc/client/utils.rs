// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_export]
macro_rules! do_request {
    ($request:ident, $($arg:expr),+; $fd:expr) => {{
        let mut client =
            $crate::jsonrpc::client::StratisClient::connect($crate::jsonrpc::consts::RPC_SOCKADDR)?;
        if let $crate::jsonrpc::interface::StratisRet::$request(ret) = client.request(
            $crate::jsonrpc::interface::StratisParams {
                type_: $crate::jsonrpc::interface::StratisParamType::$request(
                    $($arg),+
                ),
                fd_opt: Some($fd),
            }
        )? {
            ret
        } else {
            return Err($crate::stratis::StratisError::Error(
                "Request and response types did not match".to_string(),
            ));
        }
    }};
    ($request:ident, $($arg:expr ),+) => {{
        let mut client =
            $crate::jsonrpc::client::StratisClient::connect($crate::jsonrpc::consts::RPC_SOCKADDR)?;
        if let $crate::jsonrpc::interface::StratisRet::$request(ret) = client.request(
            $crate::jsonrpc::interface::StratisParams {
                type_: $crate::jsonrpc::interface::StratisParamType::$request(
                    $($arg),+
                ),
                fd_opt: None,
            }
        )? {
            ret
        } else {
            return Err($crate::stratis::StratisError::Error(
                "Request and response types did not match".to_string(),
            ));
        }
    }};
    ($request:ident) => {{
        let mut client =
            $crate::jsonrpc::client::StratisClient::connect($crate::jsonrpc::consts::RPC_SOCKADDR)?;
        if let $crate::jsonrpc::interface::StratisRet::$request(ret) = client.request(
            $crate::jsonrpc::interface::StratisParams {
                type_: $crate::jsonrpc::interface::StratisParamType::$request,
                fd_opt: None,
            }
        )? {
            ret
        } else {
            return Err($crate::stratis::StratisError::Error(
                "Request and response types did not match".to_string(),
            ));
        }
    }};
}

#[macro_export]
macro_rules! do_request_standard {
    ($request:ident, $($arg:expr),+; $fd:expr) => {{
        let (changed, rc, rs) = $crate::do_request!($request, $($arg),+; $fd);
        if rc != 0 {
            Err($crate::stratis::StratisError::Error(rs))
        } else if !changed {
            Err($crate::stratis::StratisError::Error(
                "The requested action had no effect".to_string(),
            ))
        } else {
            Ok(())
        }
    }};
    ($request:ident, $($arg:expr ),+) => {{
        let (changed, rc, rs) = $crate::do_request!($request, $($arg),+);
        if rc != 0 {
            Err($crate::stratis::StratisError::Error(rs))
        } else if !changed {
            Err($crate::stratis::StratisError::Error(
                "The requested action had no effect".to_string(),
            ))
        } else {
            Ok(())
        }
    }};
    ($request:ident) => {{
        let (changed, rc, rs) = $crate::do_request!($request);
        if rc != 0 {
            Err($crate::stratis::StratisError::Error(rs))
        } else if !changed {
            Err($crate::stratis::StratisError::Error(
                "The requested action had no effect".to_string(),
            ))
        } else {
            Ok(())
        }
    }};
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

const SUFFIXES: &[(u64, &str)] = &[
    (60, "EiB"),
    (50, "PiB"),
    (40, "TiB"),
    (30, "GiB"),
    (20, "MiB"),
    (10, "KiB"),
    (1, "B"),
];

#[allow(clippy::cast_precision_loss)]
pub fn to_suffix_repr(size: u128) -> String {
    SUFFIXES.iter().fold(String::new(), |acc, (div, suffix)| {
        let div_shifted = 1 << div;
        if acc.is_empty() && size / div_shifted >= 1 {
            format!(
                "{:.2} {}",
                (size / div_shifted) as f64 + ((size % div_shifted) as f64 / div_shifted as f64),
                suffix
            )
        } else {
            acc
        }
    })
}
