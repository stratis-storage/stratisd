// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! destroy_pool {
    ( $s:ident; $name: ident) => {
        let entry = match $s.pools.entry($name.into()) {
            Entry::Vacant(_) => return Ok(false),
            Entry::Occupied(entry) => entry,
        };
        if !entry.get().filesystems.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("filesystems remaining on pool"
                .into())));
        };
        if !entry.get().block_devs.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("devices remaining in pool".into())));
        };
        if !entry.get().cache_devs.is_empty() {
            return Err(EngineError::Stratis(ErrorEnum::Busy("cache devices remaining in pool"
                .into())));
        };
        entry.remove();
        Ok(true)
    }
}

macro_rules! get_pool {
    ( $s:ident; $name:ident ) => {
        Ok(try!($s.pools
            .get_mut($name)
            .ok_or(EngineError::Stratis(ErrorEnum::NotFound($name.into())))))
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
            return Err(EngineError::Stratis(ErrorEnum::AlreadyExists($new_name.into())));
        } else {
            let pool = $s.pools.remove($old_name).unwrap();
            $s.pools.insert($new_name.into(), pool);
            return Ok(RenameAction::Renamed);
        };
    }
}

macro_rules! get_filesystem_by_name {
    ( $s:ident; $name:expr) => {
        $s.filesystems.get_mut($name).map(|x| x as &mut Filesystem)
    }
}

macro_rules! get_filesystem_id {
    ( $s:ident; $name:expr ) => {
        $s.filesystems.iter().find(|f| f.1.name == $name).map(|x| x.1.fs_id)
    }
}
