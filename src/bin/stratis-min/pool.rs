// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use libudev::{Context, Monitor};
use uuid::Uuid;

use libstratis::{
    engine::{CreateAction, DeleteAction, Engine, Pool, StratEngine},
    stratis::{StratisError, StratisResult},
};

use crate::print_table;

const SUFFIXES: &[(u64, &str)] = &[
    (60, "EiB"),
    (50, "PiB"),
    (40, "TiB"),
    (30, "GiB"),
    (20, "MiB"),
    (10, "KiB"),
    (1, "B"),
];

// stratis-min pool setup
pub fn pool_setup() -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;

    let ctxt = Context::new()?;
    let mtr = Monitor::new(&ctxt)?;
    let mut sock = mtr.listen()?;

    for uuid in engine.locked_pools().keys() {
        if let Err(e) = engine.unlock_pool(*uuid) {
            println!(
                "Could not unlock pool with UUID {}: {}",
                uuid.to_simple_ref(),
                e
            );
        }
    }

    while let Some(event) = sock.receive_event() {
        engine.handle_event(&event);
    }

    Ok(())
}

// stratis-min pool create
pub fn pool_create(
    name: &str,
    blockdev_paths: &[&Path],
    key_desc: Option<String>,
) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    if let CreateAction::Identity = engine.create_pool(name, blockdev_paths, None, key_desc)? {
        Err(StratisError::Error(format!(
            "Pool {} already exists as requested.",
            name
        )))
    } else {
        Ok(())
    }
}

fn to_suffix(size: u64) -> String {
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

fn name_to_uuid_and_pool<'a>(
    engine: &'a mut StratEngine,
    name: &str,
) -> StratisResult<(Uuid, &'a mut dyn Pool)> {
    let mut uuids_pools_for_name = engine
        .pools_mut()
        .into_iter()
        .filter_map(|(n, u, p)| if &*n == name { Some((u, p)) } else { None })
        .collect::<Vec<_>>();
    assert!(uuids_pools_for_name.len() <= 1);
    let (uuid, pool) = uuids_pools_for_name
        .pop()
        .ok_or_else(|| StratisError::Error("No UUID found for the given name.".to_string()))?;
    Ok((uuid, pool))
}

// stratis-min pool destroy
pub fn pool_destroy(name: &str) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, _) = name_to_uuid_and_pool(&mut engine, name)?;
    match engine.destroy_pool(uuid)? {
        DeleteAction::Identity => Err(StratisError::Error(format!(
            "Pool with name {} does not exist to be deleted.",
            name
        ))),
        _ => Ok(()),
    }
}

// stratis-min pool init-cache
pub fn pool_init_cache(name: &str, paths: &[&Path]) -> StratisResult<()> {
    let mut engine = StratEngine::initialize()?;
    let (uuid, pool) = name_to_uuid_and_pool(&mut engine, name)?;
    pool.init_cache(uuid, name, paths)?;
    Ok(())
}

fn size_string(p: &dyn Pool) -> String {
    let size = p.total_physical_size().bytes();
    let used = p.total_physical_used().ok().map(|u| u.bytes());
    let free = used.map(|u| size - u);
    format!(
        "{} / {} / {}",
        to_suffix(*size),
        match used {
            Some(u) => to_suffix(*u),
            None => "FAILURE".to_string(),
        },
        match free {
            Some(f) => to_suffix(*f),
            None => "FAILURE".to_string(),
        }
    )
}

fn properties_string(p: &dyn Pool) -> String {
    let ca = if p.has_cache() { " Ca" } else { "~Ca" };
    let cr = if p.is_encrypted() { " Cr" } else { "~Cr" };
    vec![ca, cr].join(",")
}

// stratis-min pool [list]
pub fn pool_list() -> StratisResult<()> {
    let engine = StratEngine::initialize()?;

    let pools = engine.pools();
    let name_col: Vec<_> = pools.iter().map(|(n, _, _)| n.to_string()).collect();
    let physical_col: Vec<_> = pools.iter().map(|(_, _, p)| size_string(*p)).collect();
    let properties_col: Vec<_> = pools
        .iter()
        .map(|(_, _, p)| properties_string(*p))
        .collect();
    print_table!(
        "Name", name_col, "<";
        "Total Physical", physical_col, ">";
        "Properties", properties_col, ">"
    );

    Ok(())
}
