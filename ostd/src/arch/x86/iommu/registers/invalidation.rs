use core::ptr::NonNull;

use volatile::{
    VolatileRef,
    access::{ReadOnly, ReadWrite, WriteOnly},
};

use super::ExtendedCapability;

#[derive(Debug)]
pub(super) struct InvalidationRegisters {
    pub _queue_head: VolatileRef<'static, u64, ReadOnly>,
    pub queue_tail: VolatileRef<'static, u64, ReadWrite>,
    pub queue_addr: VolatileRef<'static, u64, ReadWrite>,

    pub completion_status: VolatileRef<'static, u32, ReadWrite>,
    pub _completion_event_control: VolatileRef<'static, u32, ReadWrite>,
    pub _completion_event_data: VolatileRef<'static, u32, ReadWrite>,
    pub _completion_event_addr: VolatileRef<'static, u32, ReadWrite>,
    pub _completion_event_upper_addr: VolatileRef<'static, u32, ReadWrite>,

    pub _queue_error_record: VolatileRef<'static, u64, ReadOnly>,

    pub _invalidate_address: VolatileRef<'static, u64, WriteOnly>,
    pub iotlb_invalidate: VolatileRef<'static, u64, ReadWrite>,
}

impl InvalidationRegisters {
    pub(super) unsafe fn new(base: NonNull<u8>) -> Self {
        let offset = {
            let extended_capability =
                unsafe { VolatileRef::new_read_only(base.add(0x10).cast::<u64>()) };
            let extend_cap = ExtendedCapability::new(extended_capability.as_ptr().read());
            extend_cap.iotlb_register_offset() as usize * 16
        };

        unsafe {
            Self {
                _queue_head: VolatileRef::new_read_only(base.add(0x80).cast::<u64>()),
                queue_tail: VolatileRef::new(base.add(0x88).cast::<u64>()),
                queue_addr: VolatileRef::new(base.add(0x90).cast::<u64>()),
                completion_status: VolatileRef::new(base.add(0x9C).cast::<u32>()),
                _completion_event_control: VolatileRef::new(base.add(0xA0).cast::<u32>()),
                _completion_event_data: VolatileRef::new(base.add(0xA4).cast::<u32>()),
                _completion_event_addr: VolatileRef::new(base.add(0xA8).cast::<u32>()),
                _completion_event_upper_addr: VolatileRef::new(base.add(0xAC).cast::<u32>()),
                _queue_error_record: VolatileRef::new_read_only(base.add(0xB0).cast::<u64>()),

                _invalidate_address: VolatileRef::new_restricted(
                    WriteOnly,
                    base.add(offset).cast::<u64>(),
                ),
                iotlb_invalidate: VolatileRef::new(base.add(offset).add(0x08).cast::<u64>()),
            }
        }
    }
}
