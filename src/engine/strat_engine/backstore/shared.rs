// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Methods that are shared by the cache tier and the data tier.

use std::collections::{HashMap, HashSet};

use devicemapper::{Device, LinearDevTargetParams, LinearTargetParams, Sectors, TargetLine};

use crate::{
    engine::{
        strat_engine::{
            backstore::{blockdev::StratBlockDev, devices::BlockSizes},
            serde_structs::{BaseDevSave, Recordable},
        },
        types::DevUuid,
    },
    stratis::{StratisError, StratisResult},
};

/// struct to represent a continuous set of sectors on a disk
#[derive(Debug, Clone)]
pub struct Segment {
    /// The offset into the device where this segment starts.
    pub(super) start: Sectors,
    /// The length of the segment.
    pub(super) length: Sectors,
    /// The device the segment is within.
    pub(super) device: Device,
}

impl Segment {
    /// Create a new Segment with given attributes
    pub fn new(device: Device, start: Sectors, length: Sectors) -> Segment {
        Segment {
            start,
            length,
            device,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlkDevSegment {
    pub(super) uuid: DevUuid,
    pub(super) segment: Segment,
}

impl BlkDevSegment {
    pub fn new(uuid: DevUuid, segment: Segment) -> BlkDevSegment {
        BlkDevSegment { uuid, segment }
    }

    pub fn to_segment(&self) -> Segment {
        self.segment.clone()
    }
}

/// A structure that records the segments of devices that belong to a
/// BlockDevMgr that are allocated to a layer above. The ordering of the
/// segments in the vectors must be preserved.
#[derive(Debug)]
pub struct AllocatedAbove {
    pub(super) inner: Vec<BlkDevSegment>,
}

impl Recordable<Vec<BaseDevSave>> for AllocatedAbove {
    fn record(&self) -> Vec<BaseDevSave> {
        self.inner
            .iter()
            .map(|bseg| BaseDevSave {
                parent: bseg.uuid,
                start: bseg.segment.start,
                length: bseg.segment.length,
            })
            .collect::<Vec<_>>()
    }
}

impl AllocatedAbove {
    /// Total size in the contained segments.
    pub fn size(&self) -> Sectors {
        self.inner.iter().map(|x| x.segment.length).sum::<Sectors>()
    }

    /// Build a linear dev target table from BlkDevSegments. This is useful for
    /// calls to the devicemapper library.
    pub fn map_to_dm(&self) -> Vec<TargetLine<LinearDevTargetParams>> {
        let mut table = Vec::new();
        let mut logical_start_offset = Sectors(0);

        let segments = self
            .inner
            .iter()
            .map(|bseg| bseg.to_segment())
            .collect::<Vec<_>>();
        for segment in segments {
            let (physical_start_offset, length) = (segment.start, segment.length);
            let params = LinearTargetParams::new(segment.device, physical_start_offset);
            let line = TargetLine::new(
                logical_start_offset,
                length,
                LinearDevTargetParams::Linear(params),
            );
            table.push(line);
            logical_start_offset += length;
        }

        table
    }

    /// Append the second list of BlkDevSegments to the first, or if the last
    /// segment of the first argument is adjacent to the first segment of the
    /// second argument, merge those two together.
    /// Postcondition: left.len() + right.len() - 1 <= result.len()
    /// Postcondition: result.len() <= left.len() + right.len()
    pub fn coalesce_blkdevsegs(&mut self, right: &[BlkDevSegment]) {
        self.inner = self.inner.iter().chain(right.iter()).cloned().fold(
            Vec::with_capacity(self.inner.len() + right.len()),
            |mut collect, seg| {
                if let Some(left) = collect.last_mut() {
                    if left.uuid == seg.uuid
                        && (left.segment.start + left.segment.length == seg.segment.start)
                    {
                        left.segment.length += seg.segment.length;
                    } else {
                        collect.push(seg);
                    }
                } else {
                    collect.push(seg);
                }
                collect
            },
        );
    }

    /// A set of UUIDs of every device that is allocated from.
    #[cfg(test)]
    pub fn uuids(&self) -> HashSet<DevUuid> {
        self.inner
            .iter()
            .map(|seg| seg.uuid)
            .collect::<HashSet<DevUuid>>()
    }
}

/// A partition of blockdevs in a BlockDevMgr between those in use by
/// upper layers and those that are not.
pub struct BlockDevPartition<'a> {
    pub(super) used: Vec<(DevUuid, &'a StratBlockDev)>,
    pub(super) unused: Vec<(DevUuid, &'a StratBlockDev)>,
}

/// A summary of block sizes for a BlockDevMgr, distinguishing between used
/// and unused.
pub struct BlockSizeSummary {
    pub(super) used: HashMap<BlockSizes, HashSet<DevUuid>>,
    pub(super) unused: HashMap<BlockSizes, HashSet<DevUuid>>,
}

impl<'a> From<BlockDevPartition<'a>> for BlockSizeSummary {
    fn from(pair: BlockDevPartition<'a>) -> BlockSizeSummary {
        let mut used = HashMap::new();
        for (u, bd) in pair.used {
            used.entry(bd.blksizes())
                .or_insert_with(HashSet::default)
                .insert(u);
        }

        let mut unused: HashMap<BlockSizes, _> = HashMap::new();
        for (u, bd) in pair.unused {
            unused
                .entry(bd.blksizes())
                .or_insert_with(HashSet::default)
                .insert(u);
        }

        BlockSizeSummary { used, unused }
    }
}

impl BlockSizeSummary {
    /// Check that, as far as is known, the current arrangement of device
    /// block sizes will not cause untoward behavior during the lifetime of
    /// the pool.
    /// Returns the logical block size that will alway be used by the cap
    /// device if this size exists.
    pub fn validate(&self) -> StratisResult<BlockSizes> {
        // It is not practically possible that all the data devices in the data
        // tier or all the the cache devices in the cache tier will be
        // completely unused during stratisd's normal execution. This condition
        // is here for logical completeness and in case an unused data or cache
        // tier is made for testing.
        if self.used.is_empty() {
            return if self.unused.len() > 1 {
                let error_str = "The devices in this pool have inconsistent block sizes. This is an unpredictable situation, and could lead to umnountable file systems if the pool is extended. Consider remaking the pool using devices with consistent block sizes.".to_string();
                Err(StratisError::Msg(error_str))
            } else {
                let block_sizes = self
                    .unused
                    .keys()
                    .next()
                    .expect("returned early if unused was empty");

                Ok(*block_sizes)
            };
        }

        let largest_logical_used = self
            .used
            .keys()
            .map(|x| x.logical_sector_size)
            .max()
            .expect("returned early if used was empty");

        if self
            .unused
            .keys()
            .map(|x| x.logical_sector_size)
            .any(|s| s > largest_logical_used)
        {
            let error_str = format!("Some unused block devices in the pool have a logical sector size that is larger than the largest logical sector size ({largest_logical_used}) of any of the devices that are in use. This could lead to unmountable filesystems if the pool is extended. Consider moving your data to another pool.");
            return Err(StratisError::Msg(error_str));
        }

        let largest_physical_used = self
            .used
            .keys()
            .map(|x| x.physical_sector_size)
            .max()
            .expect("returned early if used was empty");
        if self
            .unused
            .keys()
            .map(|x| x.physical_sector_size)
            .any(|s| s > largest_physical_used)
        {
            let error_str = format!("Some unused block devices in the pool have a physical sector size that is larger than the largest physical sector size ({largest_physical_used}) of any of the devices that are in use. This could lead to unmountable filesystems if the pool is extended. Consider moving your data to another pool.");
            return Err(StratisError::Msg(error_str));
        }
        Ok(BlockSizes {
            logical_sector_size: largest_logical_used,
            physical_sector_size: largest_physical_used,
        })
    }
}

/// Given a function that translates a Stratis UUID to a device
/// number, and some metadata that describes a particular segment within
/// a device by means of its Stratis UUID, and its start and offset w/in the
/// device, return the corresponding BlkDevSegment structure.
pub fn metadata_to_segment(
    uuid_to_devno: &HashMap<DevUuid, Device>,
    base_dev_save: &BaseDevSave,
) -> StratisResult<BlkDevSegment> {
    let parent = base_dev_save.parent;
    uuid_to_devno
        .get(&parent)
        .ok_or_else(|| {
            StratisError::Msg(format!(
                "No block device corresponding to stratisd UUID {:?} found",
                &parent
            ))
        })
        .map(|device| {
            BlkDevSegment::new(
                parent,
                Segment::new(*device, base_dev_save.start, base_dev_save.length),
            )
        })
}
