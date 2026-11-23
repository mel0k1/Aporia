use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

#[derive(Debug)]
pub(crate) struct AnonDeviceId(DeviceId);

impl AnonDeviceId {
    pub(crate) fn acquire() -> Option<Self> {
        DeviceIdAllocator::singleton().allocate().map(Self::new)
    }

    pub(crate) fn id(&self) -> DeviceId {
        self.0
    }

    fn new(id: DeviceId) -> Self {
        Self(id)
    }
}

impl Drop for AnonDeviceId {
    fn drop(&mut self) {
        DeviceIdAllocator::singleton().release(self.0);
    }
}

struct DeviceIdAllocator {
    minor_allocator: Mutex<IdAlloc>,
}

impl DeviceIdAllocator {
    fn singleton() -> &'static Self {
        static SINGLETON: Once<DeviceIdAllocator> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Self {
        let mut minor_allocator = IdAlloc::with_capacity(MinorId::MAX.get() as usize + 1);
        let _ = minor_allocator.alloc_specific(0).unwrap();

        Self {
            minor_allocator: Mutex::new(minor_allocator),
        }
    }

    fn allocate(&self) -> Option<DeviceId> {
        let major = MajorId::new(0);
        let minor = self.minor_allocator.lock().alloc()?;

        Some(DeviceId::new(major, MinorId::new(minor as u32)))
    }

    fn release(&self, id: DeviceId) {
        assert!(id.is_anonymous());

        self.minor_allocator.lock().free(id.minor().get() as usize);
    }
}
