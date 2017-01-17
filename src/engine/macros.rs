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
            if $s.filesystems.remove(name.into()).is_some() {
                removed.push(name);
            };
        };
        Ok(removed)
    }
}

macro_rules! destroy_pool {
    ( $s:ident; $name: ident) => {
        let entry = match $s.pools.entry($name.into()) {
            Entry::Vacant(_) => return Ok(false),
            Entry::Occupied(entry) => entry,
        };
        if !entry.get().filesystems.is_empty() {
            return Err(EngineError::Engine(ErrorEnum::Busy, "filesystems remaining on pool".into()));
        };
        if !entry.get().block_devs.is_empty() {
            return Err(EngineError::Engine(ErrorEnum::Busy, "devices remaining in pool".into()));
        };
        if !entry.get().cache_devs.is_empty() {
            return Err(EngineError::Engine(ErrorEnum::Busy, "cache devices remaining in pool"
                .into()));
        };
        entry.remove();
        Ok(true)
    }
}

macro_rules! get_pool {
    ( $s:ident; $name:ident ) => {
        Ok(try!($s.pools
            .get_mut($name)
            .ok_or(EngineError::Engine(ErrorEnum::NotFound, $name.into()))))
    }
}

macro_rules! pools {
    ( $s:ident ) => {
        BTreeMap::from_iter($s.pools.iter_mut().map(|x| (x.0 as &str, x.1 as &mut Pool)))
    }
}

macro_rules! rename_pool {
    ( $s:ident; $old_name:ident; $new_name:ident ) => {
        if $old_name == $new_name {
            return Ok(RenameAction::Identity);
        }

        if !$s.pools.contains_key($old_name) {
            return Ok(RenameAction::NoSource);
        }

        if $s.pools.contains_key($new_name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
        } else {
            let pool = $s.pools.remove($old_name).unwrap();
            $s.pools.insert($new_name.into(), pool);
            return Ok(RenameAction::Renamed);
        };
    }
}

macro_rules! rename_filesystem {
    ( $s:ident; $old_name:ident; $new_name:ident ) => {
        if $old_name == $new_name {
            return Ok(RenameAction::Identity);
        }

        if !$s.filesystems.contains_key($old_name) {
            return Ok(RenameAction::NoSource);
        }

        if $s.filesystems.contains_key($new_name) {
            return Err(EngineError::Engine(ErrorEnum::AlreadyExists, $new_name.into()));
        } else {
            let filesystem = $s.filesystems.remove($old_name).unwrap();
            $s.filesystems.insert($new_name.into(), filesystem);
            return Ok(RenameAction::Renamed);
        };
    }
}
