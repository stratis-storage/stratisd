// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    error::Error,
    ffi::OsStr,
    path::{Component, Path},
};

use stratisd::engine::StratisDmThinId;

pub fn print_value(path: &Path, mode: &str) -> Result<(), Box<dyn Error>> {
    let mut components = path.components();

    if components.next() != Some(Component::RootDir) {
        unimplemented!();
    }
    if components.next() != Some(Component::Normal(OsStr::new("dev"))) {
        unimplemented!();
    }

    if components.next() != Some(Component::Normal(OsStr::new("mapper"))) {
        unimplemented!();
    }

    if let Some(Component::Normal(dm_name)) = components.next() {
        let thin_id_parts = dm_name
            .to_str()
            .expect("FIXME")
            .parse::<StratisDmThinId>()
            .expect("FIXME");
        if mode == "pool" {
            println!("{}", thin_id_parts.pool_uuid);
        } else if mode == "filesystem" {
            println!("{}", thin_id_parts.fs_uuid);
        } else {
            unimplemented!();
        }
    } else {
        unimplemented!();
    }

    Ok(())
}
