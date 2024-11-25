// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{
    error::Error,
    fmt::{self, Display},
};

use env_logger::Builder;
use log::{info, LevelFilter};
use serde_json::{json, Value};

use devicemapper::{Bytes, Sectors};

use stratisd::engine::{
    crypt_metadata_size, integrity_meta_space, ThinPoolSizeParams, BDA,
    DEFAULT_INTEGRITY_BLOCK_SIZE, DEFAULT_INTEGRITY_JOURNAL_SIZE, DEFAULT_INTEGRITY_TAG_SIZE,
};

// 2^FS_SIZE_START_POWER is the logical size of the smallest Stratis
// filesystem for which usage data exists in FSSizeLookup::internal, i.e.,
// 512 MiB.
const FS_SIZE_START_POWER: usize = 29;

const FS_SIZE_LOOKUP_TABLE_LEN: usize = 27;
const FS_LOGICAL_SIZE_MAX: u128 = 36_028_797_018_963_968; // 32 PiB
const FS_LOGICAL_SIZE_MIN: u128 = 536_870_912; // 512 MiB

struct FSSizeLookup {
    internal: Vec<u128>,
}

impl FSSizeLookup {
    /// Calculate a predicted usage for the given logical filesystem size.
    /// Find the index of the entry in the table such that the logical
    /// filesystem size for which the data was recorded is at least as much as
    /// the logical_size argument, but no greater than twice as much, and
    /// return the value.
    /// The formula is:
    /// predicted_size = table[(log_2(logical_size) + 1) - FS_SIZE_START_POWER]
    fn lookup(&self, logical_size: Bytes) -> Sectors {
        let raw_size = *logical_size;
        assert!(raw_size >= FS_LOGICAL_SIZE_MIN);
        assert!(raw_size < FS_LOGICAL_SIZE_MAX);
        // At FS_LOGICAL_SIZE_MAX there is an 8 integer range of floating
        // point value that the u128 value may occupy. Given the large size
        // of the number and that it is the log that is being taken this is
        // of no concern for the correctness of the value.
        #[allow(clippy::cast_precision_loss)]
        let lg = f64::log2(raw_size as f64);
        // The value of lg is in the double decimal digits; truncation is
        // impossible.
        #[allow(clippy::cast_possible_truncation)]
        let result = f64::floor(lg) as usize + 1;
        let index = result - FS_SIZE_START_POWER;

        // The values are in Sectors, they are real values from a test
        // so they must be realistic in that sense.
        Bytes(self.internal[index]).sectors()
    }

    // Returns a table of recorded usage for filesystems of increasing
    // logical size. The first entry corresponds to the usage recorded for a
    // filesystem of logical size 2^FS_SIZE_START_POWER, which is 512 MiB.
    // Each subsequent value represents the usage recorded for a filesystem
    // double the size of the previous one in the list.
    fn new() -> Self {
        let internal = vec![
            20_971_520,
            20_971_520,
            22_020_096,
            23_068_672,
            23_068_672,
            31_457_280,
            34_603_008,
            51_380_224,
            84_934_656,
            152_043_520,
            286_261_248,
            571_473_920,
            1_108_344_832,
            2_171_600_896,
            2_171_600_896,
            2_171_600_896,
            2_171_600_896,
            2_205_155_328,
            2_273_312_768,
            2_407_530_496,
            2_675_965_952,
            3_212_836_864,
            4_286_578_688,
            6_434_062_336,
            10_729_029_632,
            19_318_964_224,
            36_498_833_408,
        ];
        assert!(internal.len() == FS_SIZE_LOOKUP_TABLE_LEN);
        FSSizeLookup { internal }
    }
}

#[derive(Debug)]
struct PredictError(String);

impl Display for PredictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for PredictError {}

// Get a prediction of filesystem size given a list of filesystem sizes and
// whether or not the pool allows overprovisioning.
fn get_filesystem_prediction(
    overprovisioned: bool,
    filesystem_sizes: Vec<Bytes>,
) -> Result<Sectors, Box<dyn Error>> {
    filesystem_sizes
        .iter()
        .map(|&val| {
            if !(FS_LOGICAL_SIZE_MIN..FS_LOGICAL_SIZE_MAX).contains(&val) {
                Err(Box::new(PredictError(format!(
                    "Specified filesystem size {val} is not within allowed limits."
                ))))
            } else if val.sectors().bytes() != val {
                Err(Box::new(PredictError(format!(
                    "Specified filesystem size {val} is not a multiple of sector size, 512."
                ))))
            } else {
                Ok(val)
            }
        })
        .collect::<Result<Vec<Bytes>, _>>()?;

    Ok(if overprovisioned {
        let lookup = FSSizeLookup::new();

        filesystem_sizes.iter().map(|&sz| lookup.lookup(sz)).sum()
    } else {
        filesystem_sizes.iter().map(|sz| sz.sectors()).sum()
    })
}

// Print predicted filesystem usage
pub fn predict_filesystem_usage(
    overprovisioned: bool,
    filesystem_sizes: Vec<Bytes>,
    log_level: LevelFilter,
) -> Result<(), Box<dyn Error>> {
    Builder::new().filter(None, log_level).init();

    let fs_used = get_filesystem_prediction(overprovisioned, filesystem_sizes)?;

    let used_size_str = Value::String((*(fs_used.bytes())).to_string());

    let json = json! {
        {"used": used_size_str}
    };

    println!("{json}");

    Ok(())
}

fn predict_pool_metadata_usage(
    device_sizes: Vec<Sectors>,
    journal_size: Option<Sectors>,
    tag_size: Option<Bytes>,
) -> Result<Sectors, Box<dyn Error>> {
    let stratis_metadata_alloc = BDA::default().extended_size().sectors();
    let stratis_avail_sizes = device_sizes
        .iter()
        .map(|&s| {
            info!("Total size of device: {:}", s);

            let integrity_deduction = integrity_meta_space(
                s,
                journal_size.unwrap_or(DEFAULT_INTEGRITY_JOURNAL_SIZE.sectors()),
                DEFAULT_INTEGRITY_BLOCK_SIZE,
                tag_size.unwrap_or(DEFAULT_INTEGRITY_TAG_SIZE),
            );
            info!(
                "Deduction for stratis metadata: {:}",
                stratis_metadata_alloc
            );

            info!("Deduction for integrity space: {:}", integrity_deduction);
            (*s).checked_sub(*stratis_metadata_alloc)
                .and_then(|r| r.checked_sub(*integrity_deduction))
                .map(Sectors)
                .map(|x| {
                    info!("Size after deductions: {:}", x);
                    x
                })
                .ok_or_else(|| {
                    Box::new(PredictError(
                        "Some device sizes too small for DM metadata.".into(),
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let crypt_metadata_size = crypt_metadata_size();
    let crypt_metadata_size_sectors = crypt_metadata_size.sectors();
    // verify that crypt metadata size is divisible by sector size
    assert_eq!(crypt_metadata_size_sectors.bytes(), crypt_metadata_size);

    Ok(stratis_avail_sizes.iter().cloned().sum::<Sectors>() - crypt_metadata_size_sectors)
}

// Predict usage for a newly created pool given information about whether
// or not the pool is encrypted, a list of device sizes, and an optional list
// of filesystem sizes.
pub fn predict_pool_usage(
    overprovisioned: bool,
    device_sizes: Vec<Bytes>,
    filesystem_sizes: Option<Vec<Bytes>>,
    journal_size: Option<Sectors>,
    tag_size: Option<Bytes>,
    log_level: LevelFilter,
) -> Result<(), Box<dyn Error>> {
    Builder::new().filter(None, log_level).init();

    let fs_used = filesystem_sizes
        .map(|sizes| get_filesystem_prediction(overprovisioned, sizes))
        .transpose()?;

    let device_sizes = device_sizes.iter().map(|s| s.sectors()).collect::<Vec<_>>();
    let total_size: Sectors = device_sizes.iter().cloned().sum();

    let non_metadata_size = predict_pool_metadata_usage(device_sizes, journal_size, tag_size)?;

    let size_params = ThinPoolSizeParams::new(non_metadata_size)?;
    let total_non_data = 2usize * size_params.meta_size() + size_params.mdv_size();

    let avail_size = (non_metadata_size)
        .checked_sub(*total_non_data)
        .map(Sectors)
        .ok_or_else(|| {
            Box::new(PredictError(
                "Sum of all device sizes too small for a Stratis pool.".into(),
            ))
        })?;

    let avail_size = (*avail_size)
        .checked_sub(*fs_used.unwrap_or(Sectors(0)))
        .map(Sectors)
        .ok_or_else(|| {
            Box::new(PredictError(
                "Filesystems will take up too much space on specified pool.".into(),
            ))
        })?;

    let used_size = total_size - avail_size;

    let total_size_str = Value::String((*(total_size.bytes())).to_string());
    let used_size_str = Value::String((*(used_size.bytes())).to_string());
    let avail_size_str = Value::String((*(avail_size.bytes())).to_string());
    let stratis_admin_str = Value::String((*(total_non_data.bytes())).to_string());
    let stratis_metadata_str =
        Value::String((*((total_size - non_metadata_size).bytes())).to_string());

    let json = json! {
        {"total": total_size_str, "used": used_size_str, "free": avail_size_str, "stratis-admin-space": stratis_admin_str, "stratis-metadata-space": stratis_metadata_str}
    };

    println!("{json}");

    Ok(())
}
