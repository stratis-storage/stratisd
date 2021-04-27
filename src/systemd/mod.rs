// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod bindings;

use std::{collections::HashMap, ffi::CString, io};

use log::Record;

use crate::stratis::{StratisError, StratisResult};

fn serialize_pairs(pairs: HashMap<String, String>) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", key, value))
        .fold(String::new(), |mut string, key_value_pair| {
            string += key_value_pair.as_str();
            string += "\n";
            string
        })
}

/// Notify systemd that a daemon has started.
#[allow(clippy::implicit_hasher)]
pub fn notify(unset_variable: bool, key_value_pairs: HashMap<String, String>) -> StratisResult<()> {
    let serialized_pairs = serialize_pairs(key_value_pairs);
    let cstring = CString::new(serialized_pairs)?;
    let ret = unsafe { bindings::sd_notify(unset_variable as libc::c_int, cstring.as_ptr()) };
    if ret < 0 {
        Err(StratisError::Io(io::Error::from_raw_os_error(-ret)))
    } else {
        Ok(())
    }
}

/// Send a message to the system log generated from the Rust log crate Record input.
pub fn syslog(record: &Record<'_>) {
    let cstring = match CString::new(record.args().to_string()) {
        Ok(s) => s,
        Err(_) => return,
    };
    unsafe { bindings::syslog(record.level() as libc::c_int, cstring.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_value_pair_serialization() {
        let mut hash_map = HashMap::new();
        hash_map.insert("READY".to_string(), "1".to_string());
        assert_eq!("READY=1\n".to_string(), serialize_pairs(hash_map));
    }
}
