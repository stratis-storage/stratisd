// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Utilities to support Stratis.
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use libudev;

use super::device::is_stratis_device;
use stratis::StratisResult;

/// Takes a libudev device entry and returns the properties as a HashMap.
fn device_as_map(device: &libudev::Device) -> HashMap<String, String> {
    let rc: HashMap<_, _> = device
        .properties()
        .map(|i| {
            (
                String::from(i.name().to_str().expect("Unix is utf-8")),
                String::from(i.value().to_str().expect("Unix is utf-8")),
            )
        })
        .collect();
    rc
}

/// Common function used to retrieve the udev db entry for a block device as a HashMap when found
pub fn get_udev_block_device(
    dev_node_search: &Path,
) -> StratisResult<Option<HashMap<String, String>>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    let result = enumerator
        .scan_devices()?
        .find(|x| x.devnode().map_or(false, |d| dev_node_search == d))
        .and_then(|dev| Some(device_as_map(&dev)));
    Ok(result)
}

/// Lookup the WWN from the udev db using the device node eg. /dev/sda
pub fn hw_lookup(dev_node_search: &Path) -> StratisResult<Option<String>> {
    let dev = get_udev_block_device(dev_node_search)?;
    Ok(dev.and_then(|dev| dev.get("ID_WWN").and_then(|i| Some(i.clone()))))
}

/// Collect paths for all the devices that appear to be empty from a udev db perspective.
fn get_all_empty_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;

    Ok(enumerator
        .scan_devices()?
        .filter(|dev| {
            !((dev.property_value("ID_PART_TABLE_TYPE").is_some()
                && dev.property_value("ID_PART_ENTRY_DISK").is_none())
                || dev.property_value("ID_FS_USAGE").is_some())
        })
        .map(|i| i.devnode().expect("block devices have devnode").into())
        .collect())
}

/// Retrieve all the block devices on the system that have a Stratis signature.
pub fn get_stratis_block_devices() -> StratisResult<Vec<PathBuf>> {
    let context = libudev::Context::new()?;
    let mut enumerator = libudev::Enumerator::new(&context)?;
    enumerator.match_subsystem("block")?;
    enumerator.match_property("ID_FS_TYPE", "stratis")?;

    let devices: Vec<PathBuf> = enumerator
        .scan_devices()?
        .map(|x| x.devnode().expect("block devices have devnode").into())
        .collect();

    if devices.is_empty() {
        // Either we don't have any stratis devices or we are using a distribution that doesn't
        // have a version of libblkid that supports stratis, lets make sure.
        // TODO: At some point in the future we can remove this and just return the devices.

        Ok(get_all_empty_devices()?
            .into_iter()
            .filter(|x| is_stratis_device(&x).ok().is_some())
            .collect())
    } else {
        Ok(devices)
    }
}

/// Read the amount specified, making sure we read on sector boundaries to an aligned buffer with
/// a read amount that is multiples of sector size.
pub fn read_exact_aligned<F>(
    f: &mut F,
    buf: &mut [u8],
    amount: usize,
    physical_block_size: u64,
) -> io::Result<()>
where
    F: Read + Seek,
{
    // TODO We need the pyhsical sector size from the device for this to work
    const MEMORY_ALIGNMENT: u64 = 4096;

    let current_location = f.seek(SeekFrom::Current(0))?;

    /// Round up to the specified amount
    fn r_up(position: u64, amount: u64) -> u64 {
        if position % amount == 0 {
            position
        } else {
            (position + amount) - (position % amount)
        }
    }

    /// Round down to the specified amount
    fn r_down(position: u64, amount: u64) -> u64 {
        position - (position % amount)
    }

    // Move the file position back to align with a sector size.
    let stream_offset = current_location - r_down(current_location, physical_block_size);

    // Place the stream on a sector boundary
    f.seek(SeekFrom::Start(current_location - stream_offset))?;

    // Round the amount to be read to multiple of SECTOR_SIZE, remembering to add in the offset too
    let read_size = r_up(amount as u64 + stream_offset, physical_block_size);

    // Align the buffer to be used to read the data
    let raw_buff_size = (amount + stream_offset as usize) + 2 * MEMORY_ALIGNMENT as usize;
    let mut buf_read = vec![0u8; raw_buff_size];
    let base_addr = (&buf_read[0] as *const _) as u64;
    let aligned_addr = (base_addr + MEMORY_ALIGNMENT - 1) & (!(MEMORY_ALIGNMENT - 1));

    let start: usize = (aligned_addr - base_addr) as usize;
    let end: usize = start + read_size as usize;

    f.read_exact(&mut buf_read[start..end])?;

    // Copy the data to the output buffer
    for i in 0..amount {
        buf[i] = buf_read[start + stream_offset as usize + i];
    }

    // Put the file position where is should be
    f.seek(SeekFrom::Start(current_location + amount as u64))?;

    Ok(())
}
