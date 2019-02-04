// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
extern crate decimal;
extern crate unicode_width;

extern crate clap;
extern crate libstratis;
extern crate varlink;

use std::cmp;
use std::f64;
use std::io;
use std::str::FromStr;
use std::{i128, i64};

use clap::{App, Arg, SubCommand};
use unicode_width::UnicodeWidthStr;
use varlink::Connection;

use libstratis::engine::{BlockDevState, BlockDevTier};
use libstratis::stratis::VERSION;
use libstratis::varlink_api::{ErrorKind, VarlinkClient, VarlinkClientInterface};

fn print_table(column_headings: Vec<&str>, row_entries: Vec<Vec<String>>, alignment: Vec<&str>) {
    let num_columns = column_headings.len();

    assert_eq!(num_columns, alignment.len());

    let column_lengths: Vec<usize> = (0..num_columns)
        .map(|i| {
            cmp::max(
                UnicodeWidthStr::width(column_headings[i]),
                row_entries
                    .iter()
                    .map(|r| UnicodeWidthStr::width(r[i].as_str()))
                    .max()
                    .unwrap_or(0),
            )
        })
        .collect();

    fn format_item(i: &str, column_width: usize, alignment: &str) -> String {
        let uw = UnicodeWidthStr::width(i);
        let cc = i.chars().count();
        let mut t_cw = column_width;

        t_cw -= uw - cc;

        match alignment {
            ">" => format!("{txt:>width$}", txt = i, width = t_cw),
            "<" => format!("{txt:<width$}", txt = i, width = t_cw),
            "^" => format!("{txt:^width$}", txt = i, width = t_cw),
            _ => format!("{txt:width$}", txt = i, width = t_cw),
        }
    }

    for i in 0..num_columns {
        print!(
            "{}  ",
            format_item(column_headings[i], column_lengths[i], alignment[i])
        );
    }
    println!();

    for row in row_entries {
        for i in 0..num_columns {
            print!(
                "{}  ",
                format_item(&row[i], column_lengths[i], alignment[i])
            );
        }
        println!();
    }
}

fn bytes_to_human(size: i128) -> String {
    let abs_size = if size < 0 { -size } else { size };

    fn format_num(s_value: String, unit: &str) -> String {
        let mut val = s_value
            .split('.')
            .map(String::from)
            .collect::<Vec<String>>();
        if val.len() > 1 {
            if val[1].len() > 2 {
                val[1].truncate(2);
            } else {
                val[1].push('0');
            }
            format!("{}.{} {}", val[0], val[1], unit)
        } else {
            format!("{} {}", s_value, unit)
        }
    }

    // We can go up to 8EiB using f64 support
    if abs_size <= i128::from(i64::MAX) {
        let (width, unit, result) = if abs_size >= 1_152_921_504_606_846_976 {
            (2, "EiB", abs_size as f64 / 1_152_921_504_606_846_976.0)
        } else if abs_size >= 1_125_899_906_842_624 {
            (2, "PiB", abs_size as f64 / 1_125_899_906_842_624.0)
        } else if abs_size >= 1_099_511_627_776 {
            (2, "TiB", abs_size as f64 / 1_099_511_627_776.0)
        } else if abs_size >= 1_073_741_824 {
            (2, "GiB", abs_size as f64 / 1_073_741_824.0)
        } else if abs_size >= 1_048_576 {
            (2, "MiB", abs_size as f64 / 1_048_576.0)
        } else if abs_size >= 1024 {
            (2, "KiB", abs_size as f64 / 1024.0)
        } else {
            (0, "B", abs_size as f64)
        };

        if result.fract() != 0.0 {
            format!("{:.*} {}", width, result, unit)
        } else {
            format!("{} {}", result, unit)
        }
    } else {
        //Use d128 support for larger numbers
        let c_size = decimal::d128::from_str(&format!("{}", abs_size)).expect("i128 in domain");
        let (unit, result) = if abs_size >= 1_208_925_819_614_629_174_706_176 {
            ("YiB", c_size / d128!(1_208_925_819_614_629_174_706_176))
        } else if abs_size >= 1_180_591_620_717_411_303_424 {
            ("ZiB", c_size / d128!(1_180_591_620_717_411_303_424))
        } else {
            ("EiB", c_size / d128!(1_152_921_504_606_846_975))
        };
        // Unable to use number of decimal places in format string with d128 type ?
        format_num(format!("{}", result), unit)
    }
}

fn get_connection() -> VarlinkClient {
    match Connection::with_address("unix:@stratis-storage1") {
        Err(ref e) => {
            match e.kind() {
                varlink::ErrorKind::Io(which) => match which {
                    io::ErrorKind::ConnectionRefused => println!("Error: Daemon isn't running!"),
                    _ => {
                        println!("Socket error {:?}", which);
                    }
                },
                _ => {
                    println!("Unhandled error: {:?}", e);
                }
            }
            ::std::process::exit(2);
        }
        Ok(connection) => VarlinkClient::new(connection),
    }
}

fn daemon_version() -> String {
    let mut c = get_connection();
    c.version().call().unwrap().r#version
}

fn report_error(e: &ErrorKind) {
    /*
    Io_Error(::std::io::ErrorKind),
    SerdeJson_Error(serde_json::error::Category),
    Varlink_Error,
    VarlinkReply_Error(varlink::Reply),
    Generic,
    BaseError(Option<BaseError_Args>),
    */
    match e {
        libstratis::varlink_api::ErrorKind::Io_Error(which) => match which {
            io::ErrorKind::ConnectionRefused => println!("Error: Daemon isn't running!"),
            io::ErrorKind::BrokenPipe => println!("Permission denied (got root?)"),
            _ => {
                println!("Socket error {:?}", which);
            }
        },
        libstratis::varlink_api::ErrorKind::BaseError(base) => {
            println!("Service error {:?}", base);
        }
        _ => {
            println!("Unhandled error: {:?}", e);
        }
    }
}

fn list_pools() {
    match get_connection().pools().call() {
        Ok(pools) => {
            let column_hdr = ["Name", "Total Physical Size", "Total Physical Used"];
            let alignment = ["<", ">", ">"];
            let mut rows = Vec::new();
            for p in pools.r#pools {
                let mut row = Vec::new();
                row.push(p.name);
                row.push(bytes_to_human(
                    i128::from_str(&p.total_physical_size).unwrap() * 512,
                ));
                row.push(bytes_to_human(
                    i128::from_str(&p.total_pyhsical_used).unwrap() * 512,
                ));
                rows.push(row);
            }
            rows.sort();
            print_table(column_hdr.to_vec(), rows, alignment.to_vec());
        }
        Err(e) => {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    }
}

fn get_pool_uuid(c: &mut VarlinkClient, name: &str) -> Option<String> {
    match c.pools().call() {
        Ok(pools) => pools
            .r#pools
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.uuid.clone()),
        Err(e) => {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    }
}

fn get_pool_uuid_fs_uuids(
    c: &mut VarlinkClient,
    pool_name: &str,
    fs_names: Vec<String>,
) -> (String, Vec<String>) {
    match c.pools().call() {
        Ok(pool) => {
            let pool_uuid;
            let mut fs_uuids = Vec::new();
            if let Some(p) = pool.r#pools.iter().find(|p| p.name == pool_name) {
                pool_uuid = p.uuid.clone();
                for fs_name in fs_names {
                    // TODO: This will be ugly when the number of FS gets large, we should change
                    // the API to return a map of names of structures to improve lookups or
                    // convert list of structures to a hash map lookup, or we could change the API
                    // to pass names to the daemon instead of UUIDs and let the service do all
                    // the verification/lookups which it already does for the UUIDs anyway.

                    if let Some(fs_uuid) = p
                        .file_systems
                        .iter()
                        .find(|f| f.name == fs_name)
                        .map(|f| f.uuid.clone())
                    {
                        fs_uuids.push(fs_uuid);
                    } else {
                        println!("Filesystem {} not found in pool {}!", fs_name, pool_name);
                        ::std::process::exit(4);
                    }
                }
            } else {
                println!("Pool {} not found!", pool_name);
                ::std::process::exit(4);
            }
            (pool_uuid, fs_uuids)
        }
        Err(e) => {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    }
}

fn list_blockdevs() {
    match get_connection().pools().call() {
        Ok(pools) => {
            let column_hdr = ["Pool Name", "Device Node", "Physical Size", "State", "Tier"];
            let alignment = ["<", "<", ">", "<", "<"];
            let mut rows = Vec::new();
            for p in pools.r#pools {
                for b in p.block_devs {
                    let mut row = Vec::new();
                    row.push(p.name.clone());
                    row.push(b.devnode);
                    row.push(bytes_to_human(
                        i128::from_str(&b.total_physical_size).unwrap() * 512,
                    ));
                    row.push(BlockDevState::from_i64(b.state).unwrap().to_string());
                    row.push(BlockDevTier::from_i64(b.tier).unwrap().to_string());

                    rows.push(row);
                }
            }
            rows.sort();
            print_table(column_hdr.to_vec(), rows, alignment.to_vec());
        }
        Err(e) => {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    }
}

fn list_filesystems() {
    match get_connection().pools().call() {
        Ok(pools) => {
            let column_hdr = ["Pool Name", "Name", "Used", "Created", "Device", "UUID"];
            let alignment = ["<", "<", "<", "<", "<", "<"];
            let mut rows = Vec::new();
            for p in pools.r#pools {
                for fs in p.file_systems {
                    let mut row = Vec::new();
                    row.push(p.name.clone());
                    row.push(fs.name);
                    row.push(bytes_to_human(i128::from_str(&fs.used).unwrap()));
                    row.push(fs.created);
                    row.push(fs.devnode);
                    row.push(fs.uuid);

                    rows.push(row);
                }
            }
            rows.sort();
            print_table(column_hdr.to_vec(), rows, alignment.to_vec());
        }
        Err(e) => {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    }
}

fn create_pool(pool_name: String, block_devices: Vec<String>) {
    if let Err(e) = get_connection()
        .pool_create(pool_name, None, block_devices)
        .call()
    {
        report_error(e.kind());
        ::std::process::exit(3);
    }
}

fn destroy_pool(pool_name: String) {
    let mut c = get_connection();
    if let Some(uuid) = get_pool_uuid(&mut c, &pool_name) {
        if let Err(e) = c.pool_destroy(uuid).call() {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    } else {
        println!("Pool {} not found!", pool_name);
        ::std::process::exit(4);
    }
}

fn add_cache_pool(pool_name: String, block_devices: Vec<String>) {
    let mut c = get_connection();
    if let Some(uuid) = get_pool_uuid(&mut c, &pool_name) {
        if let Err(e) = c.pool_cache_add(uuid, block_devices).call() {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    } else {
        println!("Pool {} not found!", pool_name);
        ::std::process::exit(4);
    }
}

fn add_data_pool(pool_name: String, block_devices: Vec<String>) {
    let mut c = get_connection();
    if let Some(uuid) = get_pool_uuid(&mut c, &pool_name) {
        if let Err(e) = c.pool_devs_add(uuid, block_devices).call() {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    } else {
        println!("Pool {} not found!", pool_name);
        ::std::process::exit(4);
    }
}

fn rename_pool(pool_name: String, new_name: String) {
    let mut c = get_connection();
    if let Some(uuid) = get_pool_uuid(&mut c, &pool_name) {
        if let Err(e) = c.pool_name_set(uuid, new_name).call() {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    } else {
        println!("Pool {} not found!", pool_name);
        ::std::process::exit(4);
    }
}

fn create_fs(pool_name: String, fs_names: Vec<String>) {
    let mut c = get_connection();
    if let Some(uuid) = get_pool_uuid(&mut c, &pool_name) {
        if let Err(e) = c.file_system_create(uuid, fs_names).call() {
            report_error(e.kind());
            ::std::process::exit(3);
        }
    } else {
        println!("Pool {} not found!", pool_name);
        ::std::process::exit(4);
    }
}

fn destroy_fs(pool_name: String, fs_names: Vec<String>) {
    let mut c = get_connection();
    let (pool_uuid, fs_uuids) = get_pool_uuid_fs_uuids(&mut c, &pool_name, fs_names);
    if let Err(e) = c.file_system_destroy(pool_uuid, fs_uuids).call() {
        report_error(e.kind());
        ::std::process::exit(3);
    }
}

fn rename_fs(pool_name: String, fs_names: String, new_fs_name: String) {
    let mut c = get_connection();
    let (pool_uuid, fs_uuids) = get_pool_uuid_fs_uuids(&mut c, &pool_name, [fs_names].to_vec());
    if let Err(e) = c
        .file_system_name_set(pool_uuid, fs_uuids[0].clone(), new_fs_name)
        .call()
    {
        report_error(e.kind());
        ::std::process::exit(3);
    }
}

fn snapshot_fs(pool_name: String, fs_names: String, ss_name: String) {
    let mut c = get_connection();
    let (pool_uuid, fs_uuids) = get_pool_uuid_fs_uuids(&mut c, &pool_name, [fs_names].to_vec());
    if let Err(e) = c
        .file_system_snapshot(pool_uuid, fs_uuids[0].clone(), ss_name)
        .call()
    {
        report_error(e.kind());
        ::std::process::exit(3);
    }
}

fn main() {
    let pool_name = Arg::with_name("pool_name")
        .help("Name of pool")
        .takes_value(true)
        .required(true);

    let block_devs = Arg::with_name("blockdevs")
        .help("Specify one or more blockdevs")
        .required(true)
        .multiple(true);

    let pool_operations = SubCommand::with_name("pool")
        .about("Perform General Pool Actions")
        .subcommand(
            SubCommand::with_name("create")
                .help("Create a pool")
                .about("Create a pool")
                .args(&[pool_name.clone(), block_devs.clone()]),
        )
        .subcommand(
            SubCommand::with_name("list")
                .help("List pools")
                .about("List pools"),
        )
        .subcommand(
            SubCommand::with_name("destroy")
                .help("Destroy a pool")
                .about("Destroy a pool")
                .args(&[pool_name.clone()]),
        )
        .subcommand(
            SubCommand::with_name("rename")
                .help("Rename a pool")
                .about("Rename a pool")
                .args(&[
                    pool_name.clone(),
                    Arg::with_name("new_name")
                        .help("New name of pool")
                        .takes_value(true)
                        .required(true),
                ]),
        )
        .subcommand(
            SubCommand::with_name("add-data")
                .help("Rename a pool")
                .about("Add one or more blockdevs to an existing pool for use as data storage")
                .args(&[pool_name.clone(), block_devs.clone()]),
        )
        .subcommand(
            SubCommand::with_name("add-cache")
                .help("Rename a pool")
                .about("Rename a pool")
                .args(&[pool_name.clone(), block_devs.clone()]),
        );

    let fs_operations = SubCommand::with_name("filesystem")
        .about("Commands related to filesystem(s) allocated from a pool")
        .alias("fs")
        .subcommand(
            SubCommand::with_name("create")
                .help("Create filesystem(s)")
                .about("Create filesystem(s) from a pool")
                .args(&[
                    pool_name.clone(),
                    Arg::with_name("fs_name")
                        .help("Name of fs(s) to create")
                        .takes_value(true)
                        .multiple(true)
                        .required(true),
                ]),
        )
        .subcommand(
            SubCommand::with_name("list")
                .help("List file systems")
                .about("List pools"),
        )
        .subcommand(
            SubCommand::with_name("destroy")
                .help("Destroy a file system")
                .about("Destroy the named filesystem(s) in this pool")
                .args(&[
                    pool_name.clone(),
                    Arg::with_name("fs_name")
                        .help("Name of fs(s) to destroy")
                        .takes_value(true)
                        .multiple(true)
                        .required(true),
                ]),
        )
        .subcommand(
            SubCommand::with_name("rename")
                .help("Rename a filesystem")
                .about("Rename a filesystem")
                .args(&[
                    pool_name.clone(),
                    Arg::with_name("fs_name")
                        .help("Name of the filesystem to change")
                        .takes_value(true)
                        .required(true),
                    Arg::with_name("new_name")
                        .help("New name to give that filesystem")
                        .takes_value(true)
                        .required(true),
                ]),
        )
        .subcommand(
            SubCommand::with_name("snapshot")
                .help("Snapshot a filesystem")
                .about("Snapshot the named filesystem in a pool")
                .args(&[
                    pool_name.clone(),
                    Arg::with_name("origin_name")
                        .help("Name of the filesystem to snapshot")
                        .takes_value(true)
                        .required(true),
                    Arg::with_name("snapshot_name")
                        .help("Name of the snapshot")
                        .takes_value(true)
                        .required(true),
                ]),
        );

    let blockdev_operations = SubCommand::with_name("blockdev")
        .about("Commands related to block devices that make up the pool(s)")
        .subcommand(
            SubCommand::with_name("list")
                .help("List blockdevs")
                .about("List blockdevs"),
        );

    let daemon_operations = SubCommand::with_name("daemon")
        .about("Stratis daemon information")
        .subcommand(
            SubCommand::with_name("version")
                .help("Daemon version")
                .about("version of stratisd daemon"),
        )
        .subcommand(
            SubCommand::with_name("redundancy")
                .help("Daemon version")
                .about("Redundancy designations understood by stratisd daemon"),
        );

    let matches = App::new("Stratis Storage Manager")
        .version(VERSION)
        .subcommand(pool_operations)
        .subcommand(blockdev_operations)
        .subcommand(daemon_operations)
        .subcommand(fs_operations)
        .get_matches();

    match matches.subcommand() {
        ("pool", Some(arg_matches)) => match arg_matches.subcommand() {
            ("list", Some(_)) => list_pools(),
            ("create", Some(args)) => {
                let pool_name = String::from(args.value_of("pool_name").unwrap());
                let devices: Vec<String> = args
                    .values_of("blockdevs")
                    .unwrap()
                    .map(String::from)
                    .collect();
                create_pool(pool_name, devices);
            }
            ("destroy", Some(args)) => {
                destroy_pool(String::from(args.value_of("pool_name").unwrap()));
            }
            ("add-cache", Some(args)) => {
                let pool_name = String::from(args.value_of("pool_name").unwrap());
                let devices: Vec<String> = args
                    .values_of("blockdevs")
                    .unwrap()
                    .map(String::from)
                    .collect();
                add_cache_pool(pool_name, devices);
            }
            ("add-data", Some(args)) => {
                let pool_name = String::from(args.value_of("pool_name").unwrap());
                let devices: Vec<String> = args
                    .values_of("blockdevs")
                    .unwrap()
                    .map(String::from)
                    .collect();
                add_data_pool(pool_name, devices);
            }
            ("rename", Some(args)) => {
                rename_pool(
                    String::from(args.value_of("pool_name").unwrap()),
                    String::from(args.value_of("new_name").unwrap()),
                );
            }
            _ => {
                list_pools();
            }
        },
        ("filesystem", Some(arg_matches)) => match arg_matches.subcommand() {
            ("list", Some(_)) => {
                list_filesystems();
            }
            ("create", Some(args)) => {
                let fs_names: Vec<String> = args
                    .values_of("fs_name")
                    .unwrap()
                    .map(String::from)
                    .collect();

                create_fs(String::from(args.value_of("pool_name").unwrap()), fs_names);
            }
            ("destroy", Some(args)) => {
                let fs_names: Vec<String> = args
                    .values_of("fs_name")
                    .unwrap()
                    .map(String::from)
                    .collect();

                destroy_fs(String::from(args.value_of("pool_name").unwrap()), fs_names);
            }
            ("rename", Some(args)) => {
                let pool_name = String::from(args.value_of("pool_name").unwrap());
                let fs_name = String::from(args.value_of("fs_name").unwrap());
                let new_fs_name = String::from(args.value_of("new_name").unwrap());

                rename_fs(pool_name, fs_name, new_fs_name);
            }
            ("snapshot", Some(args)) => {
                let pool_name = String::from(args.value_of("pool_name").unwrap());
                let fs_name = String::from(args.value_of("origin_name").unwrap());
                let ss_name = String::from(args.value_of("snapshot_name").unwrap());
                snapshot_fs(pool_name, fs_name, ss_name);
            }
            _ => {
                list_filesystems();
            }
        },
        ("blockdevs", Some(arg_matches)) => match arg_matches.subcommand() {
            // Only thing is "list"
            _ => {
                list_blockdevs();
            }
        },
        ("daemon", Some(arg_matches)) => {
            match arg_matches.subcommand() {
                ("version", Some(_)) => {
                    println!("{}", daemon_version());
                }
                ("redundancy", Some(_)) => {
                    // This is a hard coded constant in python.  Shouldn't we query service?
                    println!("NONE: 0");
                }
                _ => {
                    println!("{}", arg_matches.usage());
                    ::std::process::exit(1);
                }
            }
        }
        ("blockdev", Some(_)) => {
            list_blockdevs();
        }

        _ => {
            println!("{}", matches.usage());
            ::std::process::exit(1);
        }
    }
}
