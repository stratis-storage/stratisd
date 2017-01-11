// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! calculate_redundancy {
    ( $redundancy:ident ) => {
        match $redundancy {
            None => Redundancy::NONE,
            Some(n) => {
                match Redundancy::iter_variants().nth(n as usize) {
                    None => {
                        let message = format!("code {} does not correspond to any redundancy", n);
                        return Err(EngineError::Engine(ErrorEnum::Error, message));
                    }
                    Some(r) => r
                }
            }
        }
    }
}

macro_rules! destroy_filesystems {
    ( $s:ident; $fs:expr ) => {
        let mut removed = Vec::new();
        for name in $fs.iter().map(|x| *x) {
            if $s.filesystems.remove_by_name(name.into()).is_some() {
                removed.push(name);
            };
        };
        Ok(removed)
    }
}

macro_rules! destroy_pool {
    ( $s:ident; $name: ident) => {
        if let Some(ref pool) = $s.pools.get_by_name($name) {
            if !pool.filesystems.is_empty() {
                return Err(EngineError::Engine(
                    ErrorEnum::Busy, "filesystems remaining on pool".into()));
            };
        } else {
            return Ok(false);
        }
        try!($s.pools.remove_by_name($name).unwrap().destroy());
        Ok(true)
    }
}

macro_rules! get_pool {
    ( $s:ident; $name:ident ) => {
        $s.pools.get_mut_by_name($name).map(|p| p as &mut Pool)
    }
}

macro_rules! rename_pool {
    ( $s:ident; $old_name:ident; $new_name:ident ) => {
        if $old_name == $new_name {
            return Ok(RenameAction::Identity);
        }

        if !$s.pools.contains_name($old_name) {
            return Ok(RenameAction::NoSource);
        }

        if $s.pools.contains_name($new_name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
        } else {
            let mut pool = $s.pools.remove_by_name($old_name).unwrap();
            pool.rename($new_name);
            $s.pools.insert(pool);
            return Ok(RenameAction::Renamed);
        };
    }
}

macro_rules! rename_filesystem {
    ( $s:ident; $old_name:ident; $new_name:ident ) => {
        if $old_name == $new_name {
            return Ok(RenameAction::Identity);
        }

        if !$s.filesystems.contains_name($old_name) {
            return Ok(RenameAction::NoSource);
        }

        if $s.filesystems.contains_name($new_name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
        } else {
            let filesystem = $s.filesystems.remove_by_name($old_name).unwrap();
            $s.filesystems.insert(filesystem);
            return Ok(RenameAction::Renamed);
        };
    }
}
