use devicemapper::Device;

use stratis::{ErrorEnum, StratisError, StratisResult};

use super::super::serde_structs::BaseDevSave;

use super::super::super::types::DevUuid;

use super::blockdevmgr::{BlkDevSegment, Segment};

pub fn junk(
    uuid_to_devno: &Box<Fn(DevUuid) -> Option<Device>>,
    base_dev_save: &BaseDevSave,
) -> StratisResult<BlkDevSegment> {
    let parent = base_dev_save.parent;
    uuid_to_devno(parent)
        .ok_or_else(|| {
            StratisError::Engine(
                ErrorEnum::NotFound,
                format!("missign device for UUID {:?}", &parent),
            )
        })
        .map(|device| {
            BlkDevSegment::new(
                parent,
                Segment::new(device, base_dev_save.start, base_dev_save.length),
            )
        })
}
