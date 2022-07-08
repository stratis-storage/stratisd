// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{jsonrpc::client::utils::to_suffix_repr, stratis::StratisResult};

// stratis-min filesystem create
pub fn filesystem_create(pool_name: String, filesystem_name: String) -> StratisResult<()> {
    do_request_standard!(FsCreate, pool_name, filesystem_name)
}

// stratis-min filesystem [list]
pub fn filesystem_list() -> StratisResult<()> {
    let (pool_names, fs_names, used, created, paths, uuids) = do_request!(FsList);
    let used_formatted: Vec<_> = used
        .into_iter()
        .map(|u_opt| {
            u_opt
                .map(to_suffix_repr)
                .unwrap_or_else(|| "FAILURE".to_string())
        })
        .collect();
    let devices_formatted: Vec<_> = paths.into_iter().map(|p| p.display().to_string()).collect();
    let uuids_formatted: Vec<_> = uuids.into_iter().map(|u| u.to_string()).collect();
    print_table!(
        "Pool Name", pool_names, "<";
        "Name", fs_names, "<";
        "Used", used_formatted, "<";
        "Created", created, "<";
        "Device", devices_formatted, "<";
        "UUID", uuids_formatted, "<"
    );
    Ok(())
}

// stratis-min filesystem destroy
pub fn filesystem_destroy(pool_name: String, filesystem_name: String) -> StratisResult<()> {
    do_request_standard!(FsDestroy, pool_name, filesystem_name)
}

// stratis-min filesystem rename
pub fn filesystem_rename(
    pool_name: String,
    filesystem_name: String,
    new_filesystem_name: String,
) -> StratisResult<()> {
    do_request_standard!(FsRename, pool_name, filesystem_name, new_filesystem_name)
}
