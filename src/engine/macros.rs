// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! calculate_redundancy {
    ($redundancy:ident) => {
        match $redundancy {
            None | Some(0) => $crate::engine::Redundancy::NONE,
            Some(n) => {
                let message = format!("code {} does not correspond to any redundancy", n);
                return Err($crate::stratis::StratisError::Engine(
                    $crate::stratis::ErrorEnum::Error,
                    message,
                ));
            }
        }
    };
}

macro_rules! get_pool {
    ($s:ident; $uuid:ident) => {
        $s.pools
            .get_by_uuid($uuid)
            .map(|(name, p)| (name.clone(), p as &dyn $crate::engine::Pool))
    };
}

macro_rules! get_mut_pool {
    ($s:ident; $uuid:ident) => {
        $s.pools
            .get_mut_by_uuid($uuid)
            .map(|(name, p)| (name.clone(), p as &mut dyn $crate::engine::Pool))
    };
}

macro_rules! rename_pre {
    ($s:expr; $uuid:ident; $new_name:ident; $not_found:expr; $same:expr) => {{
        let old_name = match $s.get_by_uuid($uuid) {
            Some((name, _)) => name,
            None => return $not_found,
        };

        if &*old_name == $new_name {
            return $same;
        }

        if $s.contains_name($new_name) {
            return Err($crate::stratis::StratisError::Engine(
                $crate::stratis::ErrorEnum::AlreadyExists,
                $new_name.into(),
            ));
        }
        old_name
    }};
}

macro_rules! rename_filesystem_pre {
    ($s:ident; $uuid:ident; $new_name:ident) => {{
        rename_pre!(
            $s.filesystems;
            $uuid;
            $new_name;
            Err($crate::stratis::StratisError::Engine(
                $crate::stratis::ErrorEnum::NotFound,
                format!("Filesystem not found with UUID of {}", $uuid),
            ));
            Ok(None)
        )
    }}
}

macro_rules! rename_pre_idem {
    ($s:expr; $uuid:ident; $new_name:ident) => {{
        rename_pre!(
            $s;
            $uuid;
            $new_name;
            Ok($crate::engine::RenameAction::NoSource);
            Ok($crate::engine::RenameAction::Identity)
        )
    }}
}

macro_rules! rename_filesystem_pre_idem {
    ($s:ident; $uuid:ident; $new_name:ident) => {{
        rename_pre_idem!(
            $s.filesystems;
            $uuid;
            $new_name
        )
    }}
}

macro_rules! rename_pool_pre_idem {
    ($s:ident; $uuid:ident; $new_name:ident) => {{
        rename_pre_idem!(
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
