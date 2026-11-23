use alloc::boxed::Box;

use bit_field::BitField;
use spin::Once;

use crate::{
    cpu::PinCurrentCpu,
    cpu_local,
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
};

mod x2apic;
mod xapic;

static APIC_TYPE: Once<ApicType> = Once::new();

pub(in crate::arch) fn get_or_init(_guard: &dyn PinCurrentCpu) -> &(dyn Apic + 'static) {
    struct ForceSyncSend<T>(T);

    unsafe impl<T> Sync for ForceSyncSend<T> {}
    unsafe impl<T> Send for ForceSyncSend<T> {}

    impl<T> ForceSyncSend<T> {
        unsafe fn get(&self) -> &T {
            &self.0
        }
    }

    cpu_local! {
        static APIC_INSTANCE: Once<ForceSyncSend<Box<dyn Apic + 'static>>> = Once::new();
    }

    let apic_instance = APIC_INSTANCE.get_on_cpu(crate::cpu::CpuId::current_racy());

    if let Some(apic) = apic_instance.get() {
        return &**unsafe { apic.get() };
    }

    apic_instance.call_once(|| match APIC_TYPE.get().unwrap() {
        ApicType::XApic(io_mem) => {
            let mut xapic = xapic::XApic::new(io_mem).unwrap();
            xapic.enable();
            let version = xapic.version();
            crate::info!(
                "xAPIC ID: {:x}, Version: {:x}, Max LVT: {:x}",
                xapic.id(),
                version & 0xff,
                (version >> 16) & 0xff
            );
            ForceSyncSend(Box::new(xapic))
        }
        ApicType::X2Apic => {
            let mut x2apic = x2apic::X2Apic::new().unwrap();
            x2apic.enable();
            let version = x2apic.version();
            crate::info!(
                "x2APIC ID: {:x}, Version: {:x}, Max LVT: {:x}",
                x2apic.id(),
                version & 0xff,
                (version >> 16) & 0xff
            );
            ForceSyncSend(Box::new(x2apic))
        }
    });

    let apic = apic_instance.get().unwrap();
    &**unsafe { apic.get() }
}

pub(in crate::arch) trait Apic: ApicTimer {
    fn id(&self) -> u32;

    fn version(&self) -> u32;

    fn eoi(&self);

    unsafe fn send_ipi(&self, icr: Icr);
}

pub(in crate::arch) trait ApicTimer {
    fn set_timer_init_count(&self, value: u64);

    fn timer_current_count(&self) -> u64;

    fn set_lvt_timer(&self, value: u64);

    fn set_timer_div_config(&self, div_config: DivideConfig);
}

enum ApicType {
    XApic(IoMem<Sensitive>),
    X2Apic,
}

pub(in crate::arch) struct Icr(u64);

impl Icr {
    #[expect(clippy::too_many_arguments)]
    pub(in crate::arch) fn new(
        destination: ApicId,
        destination_shorthand: DestinationShorthand,
        trigger_mode: TriggerMode,
        level: Level,
        delivery_status: DeliveryStatus,
        destination_mode: DestinationMode,
        delivery_mode: DeliveryMode,
        vector: u8,
    ) -> Self {
        let dest = match destination {
            ApicId::XApic(d) => (d as u64) << 56,
            ApicId::X2Apic(d) => (d as u64) << 32,
        };
        Icr(dest
            | ((destination_shorthand as u64) << 18)
            | ((trigger_mode as u64) << 15)
            | ((level as u64) << 14)
            | ((delivery_status as u64) << 12)
            | ((destination_mode as u64) << 11)
            | ((delivery_mode as u64) << 8)
            | (vector as u64))
    }

    pub(in crate::arch) fn lower(&self) -> u32 {
        self.0 as u32
    }

    pub(in crate::arch) fn upper(&self) -> u32 {
        (self.0 >> 32) as u32
    }
}

pub(in crate::arch) enum ApicId {
    XApic(u8),
    X2Apic(u32),
}

impl ApicId {
    #[expect(unused)]
    pub(in crate::arch) fn x2apic_logical_id(&self) -> u32 {
        (self.x2apic_logical_cluster_id() << 16) | (1 << self.x2apic_logical_field_id())
    }

    pub(in crate::arch) fn x2apic_logical_cluster_id(&self) -> u32 {
        let apic_id = match *self {
            ApicId::XApic(id) => id as u32,
            ApicId::X2Apic(id) => id,
        };
        apic_id.get_bits(4..=19)
    }

    pub(in crate::arch) fn x2apic_logical_field_id(&self) -> u32 {
        let apic_id = match *self {
            ApicId::XApic(id) => id as u32,
            ApicId::X2Apic(id) => id,
        };
        apic_id.get_bits(0..=3)
    }
}

impl From<u32> for ApicId {
    fn from(value: u32) -> Self {
        match APIC_TYPE.get().unwrap() {
            ApicType::XApic(_) => ApicId::XApic(value as u8),
            ApicType::X2Apic => ApicId::X2Apic(value),
        }
    }
}

#[repr(u64)]
pub(in crate::arch) enum DestinationShorthand {
    NoShorthand = 0b00,
    #[expect(dead_code)]
    MySelf = 0b01,
    AllIncludingSelf = 0b10,
    AllExcludingSelf = 0b11,
}

#[repr(u64)]
pub(in crate::arch) enum TriggerMode {
    Edge = 0,
    Level = 1,
}

#[repr(u64)]
pub(in crate::arch) enum Level {
    Deassert = 0,
    Assert = 1,
}

#[repr(u64)]
pub(in crate::arch) enum DeliveryStatus {
    Idle = 0,
    #[expect(dead_code)]
    SendPending = 1,
}

#[repr(u64)]
pub(in crate::arch) enum DestinationMode {
    Physical = 0,
    #[expect(dead_code)]
    Logical = 1,
}

#[repr(u64)]
pub(in crate::arch) enum DeliveryMode {
    Fixed = 0b000,
    #[expect(dead_code)]
    LowestPriority = 0b001,
    #[expect(dead_code)]
    Smi = 0b010,
    _Reserved = 0b011,
    #[expect(dead_code)]
    Nmi = 0b100,
    Init = 0b101,
    StartUp = 0b110,
}

#[derive(Debug)]
pub(in crate::arch) enum ApicInitError {
    NoApic,
}

#[expect(dead_code)]
#[repr(u32)]
#[derive(Debug)]
pub(in crate::arch) enum DivideConfig {
    Divide1 = 0b1011,
    Divide2 = 0b0000,
    Divide4 = 0b0001,
    Divide8 = 0b0010,
    Divide16 = 0b0011,
    Divide32 = 0b1000,
    Divide64 = 0b1001,
    Divide128 = 0b1010,
}

pub(in crate::arch) fn init(io_mem_builder: &IoMemAllocatorBuilder) -> Result<(), ApicInitError> {
    if x2apic::X2Apic::has_x2apic() {
        crate::info!("x2APIC found!");
        APIC_TYPE.call_once(|| ApicType::X2Apic);
        Ok(())
    } else if xapic::XApic::has_xapic() {
        crate::info!("xAPIC found!");
        let base_address = unsafe { xapic::read_xapic_base_address() };
        let io_mem = io_mem_builder.reserve(
            base_address..(base_address + xapic::XAPIC_MMIO_SIZE),
            crate::mm::CachePolicy::Uncacheable,
        );
        APIC_TYPE.call_once(|| ApicType::XApic(io_mem));
        Ok(())
    } else {
        crate::warn!("Neither x2APIC nor xAPIC found!");
        Err(ApicInitError::NoApic)
    }
}
