// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! calculate_redundancy {
    ( $redundancy:ident ) => {
        match $redundancy {
            None | Some(0) => Redundancy::NONE,
            Some(n) => {
                let message = format!("code {} does not correspond to any redundancy", n);
                return Err(EngineError::Engine(ErrorEnum::Error, message));
            }
        }
    }
}

macro_rules! get_pool {
    ( $s:ident; $uuid:ident ) => {
        $s.pools.get_by_uuid($uuid).map(|p| p as &Pool)
    }
}

macro_rules! get_mut_pool {
    ( $s:ident; $uuid:ident ) => {
        $s.pools.get_mut_by_uuid($uuid).map(|p| p as &mut Pool)
    }
}

macro_rules! rename_filesystem_pre {
    ( $s:ident; $uuid:ident; $new_name:ident ) => {
        {
            let old_name = match $s.filesystems.get_by_uuid($uuid) {
                Some(filesystem) => filesystem.name().to_owned(),
                None => return Ok(RenameAction::NoSource),
            };

            if old_name == $new_name {
                return Ok(RenameAction::Identity);
            }

            if $s.filesystems.contains_name($new_name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
            }
            old_name
        }
    }
}

macro_rules! rename_pool_pre {
    ( $s:ident; $uuid:ident; $new_name:ident ) => {
        {
            let old_name = match $s.pools.get_by_uuid($uuid) {
                Some(pool) => pool.name().to_owned(),
                None => return Ok(RenameAction::NoSource),
            };

            if old_name == $new_name {
                return Ok(RenameAction::Identity);
            }

            if $s.pools.contains_name($new_name) {
                return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
            }
            old_name
        }
    }
}

macro_rules! check_engine {
    ( $s:ident ) => {
        for pool in &mut $s.pools {
            // FIXME: It is not really correct to ignore result of pool.check().
            let _ = pool.check();
        }
    }
}

macro_rules! set_blockdev_user_info {
    ( $s:ident; $info:ident ) => {
        if $s.user_info.as_ref().map(|x| &**x) != $info {
            $s.user_info = $info.map(|x| x.to_owned());
            true
        } else {
            false
        }
    }
}
