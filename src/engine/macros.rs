// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! calculate_redundancy {
    ($redundancy:ident) => {
        match $redundancy {
            None | Some(0) => $crate::engine::Redundancy::NONE,
            Some(n) => {
                let message = format!("code {} does not correspond to any redundancy", n);
                return Err($crate::stratis::StratisError::Msg(message));
            }
        }
    };
}

macro_rules! get_pool {
    ($s:ident; $key:expr) => {
        $s.pools.read($key).await
    };
}

macro_rules! get_mut_pool {
    ($s:ident; $key:expr) => {
        $s.pools.write($key).await
    };
}

macro_rules! rename_pre_sync {
    ($s:expr; $uuid:expr; $new_name:expr; $not_found:expr; $same:expr) => {{
        let old_name = match $s.get_by_uuid($uuid) {
            Some((name, _)) => name,
            None => return $not_found,
        };

        if &*old_name == $new_name {
            return $same;
        }

        if $s.contains_name($new_name) {
            return Err($crate::stratis::StratisError::Msg(format!(
                "Entry with name {} already exists",
                $new_name,
            )));
        }
        old_name
    }};
}

macro_rules! rename_pre_async {
    ($s:expr; $uuid:expr; $new_name:expr; $not_found:expr; $same:expr) => {{
        let old_name = {
            let guard = $s.read($crate::engine::types::LockKey::Uuid($uuid)).await;
            match guard.as_ref().map(|g| g.as_tuple()) {
                Some((name, _, _)) => name,
                None => return $not_found,
            }
        };

        if old_name == $new_name {
            return $same;
        }

        if $s
            .read($crate::engine::types::LockKey::Name($new_name))
            .await
            .is_some()
        {
            return Err($crate::stratis::StratisError::Msg(format!(
                "Entry with name {} already exists",
                $new_name,
            )));
        }
        old_name
    }};
}

macro_rules! rename_filesystem_pre {
    ($s:expr; $uuid:expr; $new_name:expr) => {{
        rename_pre_sync!(
            $s.filesystems;
            $uuid;
            $new_name;
            Ok(None);
            Ok(Some(false))
        )
    }}
}

macro_rules! rename_pre_idem_sync {
    ($s:expr; $uuid:expr; $new_name:expr) => {{
        rename_pre_sync!(
            $s;
            $uuid;
            $new_name;
            Ok($crate::engine::RenameAction::NoSource);
            Ok($crate::engine::RenameAction::Identity)
        )
    }}
}

macro_rules! rename_pre_idem_async {
    ($s:expr; $uuid:expr; $new_name:expr) => {{
        rename_pre_async!(
            $s;
            $uuid;
            $new_name;
            Ok($crate::engine::RenameAction::NoSource);
            Ok($crate::engine::RenameAction::Identity)
        )
    }}
}

macro_rules! rename_filesystem_pre_idem {
    ($s:expr; $uuid:expr; $new_name:expr) => {{
        rename_pre_idem_sync!(
            $s.filesystems;
            $uuid;
            $new_name
        )
    }}
}

macro_rules! rename_pool_pre_idem {
    ($s:expr; $uuid:expr; $new_name:expr) => {{
        rename_pre_idem_async!(
            $s.pools;
            $uuid;
            $new_name
        )
    }}
}

macro_rules! set_blockdev_user_info {
    ($s:ident; $info:ident) => {
        if $s.user_info.as_ref().map(|x| &**x) != $info {
            $s.user_info = $info.map(|x| x.to_owned());
            true
        } else {
            false
        }
    };
}

macro_rules! device_list_check_num {
    ($vec:ident, ($is_one:tt, $is_many:tt)) => {{
        let joined_string = $vec.join(", ");
        if $vec.len() == 1 {
            format!($is_one, joined_string)
        } else if $vec.len() > 1 {
            format!($is_many, joined_string)
        } else {
            String::new()
        }
    }};
}

macro_rules! create_pool_generate_error_string {
    ($pool_name:ident, $input:ident, $exists:ident) => {
        format!(
            "There was a difference in the blockdevs associated with \
             the existing pool named {} and the input requesting creation \
             of a pool by the same name{}{}",
            $pool_name,
            device_list_check_num!(
                $input,
                (
                    " - the input requests blockdev {} \
                     which does not exist in the current pool",
                    " - the input requests blockdevs {} \
                     which do not exist in the current pool"
                )
            ),
            device_list_check_num!(
                $exists,
                (
                    " - the existing pool contains blockdev {} which was \
                     not requested by the input",
                    " - the existing pool contains blockdevs {} which were \
                     not requested by the input"
                )
            ),
        )
    };
}

#[cfg(test)]
macro_rules! strs_to_paths {
    ($slice:expr) => {
        &$slice.iter().map(Path::new).collect::<Vec<_>>()
    };
}

macro_rules! init_cache_generate_error_string {
    ($input:ident, $exists:ident) => {
        format!(
            "The input requests initialization of a cache with different block devices \
             from the block devices in the existing cache{}{} ; to \
             resolve this error, the block devices requested in the input should be the \
             same as the block devices in the existing cache.",
            device_list_check_num!(
                $input,
                (
                    "; the existing cache contains \
                     the block device {} which the input did not include",
                    "; the existing cache contains \
                     the block devices {} which the input did not include"
                )
            ),
            device_list_check_num!(
                $exists,
                (
                    "; the input requested block device {} which does not exist in the already \
                     initialized cache",
                    "; the input requested block devices {} which do not exist in the already \
                     initialized cache"
                )
            ),
        )
    };
}

macro_rules! convert_int {
    ($expr:expr, $from_type:ty, $to_type:ty) => {{
        let expr = $expr;
        <$to_type as std::convert::TryFrom<$from_type>>::try_from(expr).map_err(|_| {
            $crate::stratis::StratisError::Msg(format!(
                "Failed to convert integer {} from {} to {}",
                expr,
                stringify!($from_type),
                stringify!($to_type)
            ))
        })
    }};
}

macro_rules! convert_const {
    ($expr:expr, $from_type:ty, $to_type:ty) => {
        <$to_type as std::convert::TryFrom<$from_type>>::try_from($expr)
            .expect(format!("{} is a constant", stringify!($expr)).as_str())
    };
}

#[cfg(test)]
macro_rules! convert_test {
    ($expr:expr, $from_type:ty, $to_type:ty) => {
        <$to_type as std::convert::TryFrom<$from_type>>::try_from($expr).unwrap()
    };
}

// Macro for formatting a Uuid object for use in a device name or in
// a signature buffer.
macro_rules! uuid_to_string {
    ($uuid:expr) => {
        $uuid.to_simple_ref().to_string()
    };
}

#[cfg(test)]
// Macro for allowing a delay for certain operations in tests
macro_rules! retry_operation {
    ($expr:expr) => {
        for i in 0.. {
            match ($expr, i) {
                (Ok(_), _) => break,
                (Err(e), i) if i == 3 => Err(e).unwrap(),
                (Err(e), _) => {
                    debug!("Waiting on {} that returned error {}", stringify!($expr), e);
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    };
}

macro_rules! pool_enc_to_enc {
    ($ei_option:expr) => {
        match $ei_option {
            Some(pei) => Some($crate::engine::types::EncryptionInfo::try_from(
                pei.clone(),
            )?),
            None => None,
        }
    };
}
