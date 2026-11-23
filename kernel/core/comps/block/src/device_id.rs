use alloc::collections::btree_set::BTreeSet;

use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

use crate::Error;

pub const MAX_MAJOR: u16 = 511;

const LAST_DYNAMIC_MAJOR: u16 = 254;

static MAJORS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

pub fn acquire_major(major: MajorId) -> Result<MajorIdOwner, Error> {
    if major.get() > MAX_MAJOR {
        return Err(Error::InvalidArgs);
    }

    if MAJORS.lock().insert(major.get()) {
        Ok(MajorIdOwner(major))
    } else {
        Err(Error::IdAcquired)
    }
}

pub fn allocate_major() -> Result<MajorIdOwner, Error> {
    let mut majors = MAJORS.lock();
    for id in (1..LAST_DYNAMIC_MAJOR + 1).rev() {
        if majors.insert(id) {
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    Err(Error::IdExhausted)
}

pub struct MajorIdOwner(MajorId);

impl MajorIdOwner {
    pub fn get(&self) -> MajorId {
        self.0
    }
}

impl Drop for MajorIdOwner {
    fn drop(&mut self) {
        MAJORS.lock().remove(&self.0.get());
    }
}

const EXTENDED_MAJOR: u16 = 259;

pub(crate) struct ExtendedDeviceIdAllocator {
    major: MajorIdOwner,
    minor_allocator: Mutex<IdAlloc>,
}

impl ExtendedDeviceIdAllocator {
    fn new() -> Self {
        let major = MajorId::new(EXTENDED_MAJOR);
        let minor_allocator = IdAlloc::with_capacity(MinorId::MAX.get() as usize + 1);

        Self {
            major: acquire_major(major).unwrap(),
            minor_allocator: Mutex::new(minor_allocator),
        }
    }

    pub(crate) fn allocate(&self) -> DeviceId {
        let minor = self.minor_allocator.lock().alloc().unwrap() as u32;

        DeviceId::new(self.major.get(), MinorId::new(minor))
    }

    #[expect(dead_code)]
    pub(crate) fn release(&mut self, id: DeviceId) {
        if id.major() != self.major.get() {
            return;
        }

        self.minor_allocator.lock().free(id.minor().get() as usize);
    }
}

pub(crate) static EXTENDED_DEVICE_ID_ALLOCATOR: Once<ExtendedDeviceIdAllocator> = Once::new();

pub(super) fn init() {
    EXTENDED_DEVICE_ID_ALLOCATOR.call_once(ExtendedDeviceIdAllocator::new);
}
