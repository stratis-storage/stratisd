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
        for uuid in $fs.iter().map(|x| *x) {
            if let Some(fs) = $s.filesystems.remove_by_uuid(&uuid) {
                try!(fs.destroy());
                removed.push(uuid);
            }
        }
        Ok(removed)
    }
}

macro_rules! destroy_pool {
    ( $s:ident; $uuid: ident) => {
        if let Some(ref pool) = $s.pools.get_by_uuid($uuid) {
            if !pool.filesystems.is_empty() {
                return Err(EngineError::Engine(
                    ErrorEnum::Busy, "filesystems remaining on pool".into()));
            };
        } else {
            return Ok(false);
        }
        try!($s.pools.remove_by_uuid($uuid)
             .expect("Must succeed since $s.pool.get_by_uuid() returned a value.")
             .destroy());
        Ok(true)
    }
}

macro_rules! get_pool {
    ( $s:ident; $uuid:ident ) => {
        $s.pools.get_mut_by_uuid($uuid).map(|p| p as &mut Pool)
    }
}

macro_rules! get_filesystem {
    ( $s:ident; $uuid:ident ) => {
        $s.filesystems.get_mut_by_uuid($uuid).map(|p| p as &mut Filesystem)
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

macro_rules! check_engine {
    ( $s:ident ) => {
        for pool in &mut $s.pools {
            pool.check();
        }
    }
}
