// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::path::Path;

use libudev::{Context, Monitor};

use libstratis::{
    engine::{Engine, Pool, StratEngine},
    stratis::StratisResult,
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
        engine.unlock_pool(*uuid)?;
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
    engine.create_pool(name, blockdev_paths, None, key_desc)?;
    Ok(())
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
