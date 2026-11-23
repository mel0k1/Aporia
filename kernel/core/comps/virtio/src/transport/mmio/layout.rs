use core::fmt::Debug;

#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct VirtioMmioLayout {
    pub magic_value: u32,
    pub version: u32,
    pub device_id: u32,
    pub vendor_id: u32,

    pub device_features: u32,
    pub device_features_select: u32,

    __r1: [u8; 8],

    pub driver_features: u32,
    pub driver_features_select: u32,

    pub legacy_guest_page_size: u32,

    __r2: [u8; 4],

    pub queue_sel: u32,
    pub queue_num_max: u32,
    pub queue_num: u32,

    pub legacy_queue_align: u32,

    pub legacy_queue_pfn: u32,

    pub queue_ready: u32,

    __r3: [u8; 8],

    pub queue_notify: u32,

    __r4: [u8; 12],

    pub interrupt_status: u32,
    pub interrupt_ack: u32,

    __r5: [u8; 8],

    pub status: u32,

    __r6: [u8; 12],

    pub queue_desc_low: u32,
    pub queue_desc_high: u32,

    __r7: [u8; 8],

    pub queue_driver_low: u32,
    pub queue_driver_high: u32,

    __r8: [u8; 8],

    pub queue_device_low: u32,
    pub queue_device_high: u32,

    __r9: [u8; 84],

    pub config_generation: u32,
}

impl Debug for VirtioMmioLayout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VirtioMmioLayout")
            .field("magic_value", &self.magic_value)
            .field("version", &self.version)
            .field("device_id", &self.device_id)
            .field("vendor_id", &self.vendor_id)
            .field("device_features", &self.device_features)
            .field("device_features_sel", &self.device_features_select)
            .field("driver_features", &self.driver_features)
            .field("driver_features_sel", &self.driver_features_select)
            .field("legacy_guest_page_size", &self.legacy_guest_page_size)
            .field("queue_sel", &self.queue_sel)
            .field("queue_num_max", &self.queue_num_max)
            .field("queue_num", &self.queue_num)
            .field("legacy_queue_align", &self.legacy_queue_align)
            .field("legacy_queue_pfn", &self.legacy_queue_pfn)
            .field("queue_ready", &self.queue_ready)
            .field("queue_notify", &self.queue_notify)
            .field("interrupt_status", &self.interrupt_status)
            .field("interrupt_ack", &self.interrupt_ack)
            .field("status", &self.status)
            .field("queue_desc_low", &self.queue_desc_low)
            .field("queue_desc_high", &self.queue_desc_high)
            .field("queue_driver_low", &self.queue_driver_low)
            .field("queue_driver_high", &self.queue_driver_high)
            .field("queue_device_low", &self.queue_device_low)
            .field("queue_device_high", &self.queue_device_high)
            .field("config_generation", &self.config_generation)
            .finish()
    }
}
