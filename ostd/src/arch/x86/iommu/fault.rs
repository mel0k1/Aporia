use alloc::vec::Vec;
use core::{fmt::Debug, ptr::NonNull};

use bitflags::bitflags;
use spin::Once;
use volatile::{VolatileRef, access::ReadWrite};

use super::registers::Capability;
use crate::{
    arch::trap::TrapFrame,
    error, info,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};

#[derive(Debug)]
struct FaultEventRegisters {
    status: VolatileRef<'static, u32, ReadWrite>,
    _control: VolatileRef<'static, u32, ReadWrite>,
    _data: VolatileRef<'static, u32, ReadWrite>,
    _address: VolatileRef<'static, u32, ReadWrite>,
    _upper_address: VolatileRef<'static, u32, ReadWrite>,
    recordings: Vec<VolatileRef<'static, u128, ReadWrite>>,

    _fault_irq: IrqLine,
}

impl FaultEventRegisters {
    fn status(&self) -> FaultStatus {
        FaultStatus::from_bits_truncate(self.status.as_ptr().read())
    }

    unsafe fn new(base_register_vaddr: NonNull<u8>) -> Self {
        let (capability, status, mut control, mut data, mut address, upper_address) = unsafe {
            let base = base_register_vaddr;
            (
                VolatileRef::new_read_only(base.add(0x08).cast::<u64>()),
                VolatileRef::new(base.add(0x34).cast::<u32>()),
                VolatileRef::new(base.add(0x38).cast::<u32>()),
                VolatileRef::new(base.add(0x3c).cast::<u32>()),
                VolatileRef::new(base.add(0x40).cast::<u32>()),
                VolatileRef::new(base.add(0x44).cast::<u32>()),
            )
        };

        let capability_val = Capability::new(capability.as_ptr().read());
        let length = capability_val.fault_recording_number() as usize + 1;
        let offset = (capability_val.fault_recording_register_offset() as usize) * 16;

        let mut recordings = Vec::with_capacity(length);
        for i in 0..length {
            recordings.push(unsafe {
                VolatileRef::new(base_register_vaddr.add(offset).add(i * 16).cast::<u128>())
            })
        }

        let mut fault_irq = IrqLine::alloc().unwrap();
        fault_irq.on_active(iommu_fault_handler);

        data.as_mut_ptr().write(fault_irq.num() as u32);
        address.as_mut_ptr().write(0xFEE0_0000);
        control.as_mut_ptr().write(0);

        FaultEventRegisters {
            status,
            _control: control,
            _data: data,
            _address: address,
            _upper_address: upper_address,
            recordings,
            _fault_irq: fault_irq,
        }
    }
}

struct FaultRecording(u128);

impl FaultRecording {
    fn is_fault(&self) -> bool {
        self.0 & (1 << 127) != 0
    }

    fn clear_fault(&mut self) {
        self.0 &= !(1 << 127);
    }

    fn request_type(&self) -> FaultRequestType {
        let t1 = ((self.0 & (1 << 126)) >> 125) as u8;
        let t2 = ((self.0 & (1 << 92)) >> 92) as u8;
        let typ = t1 + t2;
        match typ {
            0 => FaultRequestType::Write,
            1 => FaultRequestType::Page,
            2 => FaultRequestType::Read,
            3 => FaultRequestType::AtomicOp,
            _ => unreachable!(),
        }
    }

    fn address_type(&self) -> Option<FaultAddressType> {
        match (self.0 >> 124) & 3 {
            0 => Some(FaultAddressType::UntranslatedRequest),
            1 => Some(FaultAddressType::TranslationRequest),
            2 => Some(FaultAddressType::TranslatedRequest),
            3 => None,
            _ => unreachable!(),
        }
    }

    fn source_identifier(&self) -> u16 {
        ((self.0 & 0xFFFF_0000_0000_0000_0000) >> 64) as u16
    }

    fn fault_info(&self) -> u64 {
        ((self.0 & 0xFFFF_FFFF_FFFF_F000) >> 12) as u64
    }

    #[expect(dead_code)]
    fn pasid_value(&self) -> u32 {
        ((self.0 & 0x00FF_FFF0_0000_0000_0000_0000_0000_0000) >> 104) as u32
    }

    fn fault_reason(&self) -> u8 {
        ((self.0 & 0xFF_0000_0000_0000_0000_0000_0000) >> 96) as u8
    }

    #[expect(dead_code)]
    fn pasid_present(&self) -> bool {
        (self.0 & 0x8000_0000_0000_0000_0000_0000) != 0
    }

    #[expect(dead_code)]
    fn execute_permission_request(&self) -> bool {
        (self.0 & 0x4000_0000_0000_0000_0000_0000) != 0
    }

    #[expect(dead_code)]
    fn privilege_mode_request(&self) -> bool {
        (self.0 & 0x2000_0000_0000_0000_0000_0000) != 0
    }
}

impl Debug for FaultRecording {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FaultRecording")
            .field("Fault", &self.is_fault())
            .field("Request type", &self.request_type())
            .field("Address type", &self.address_type())
            .field("Source identifier", &self.source_identifier())
            .field("Fault Reason", &self.fault_reason())
            .field("Fault info", &self.fault_info())
            .field("Raw", &self.0)
            .finish()
    }
}

#[repr(u8)]
#[derive(Debug)]
enum FaultRequestType {
    Write = 0,
    Page = 1,
    Read = 2,
    AtomicOp = 3,
}

#[expect(clippy::enum_variant_names)]
#[repr(u8)]
#[derive(Debug)]
enum FaultAddressType {
    UntranslatedRequest = 0,
    TranslationRequest = 1,
    TranslatedRequest = 2,
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct FaultStatus: u32 {
        const PFO = 1 << 0;
        const PPF = 1 << 1;
        const IQE = 1 << 4;
        const ICE = 1 << 5;
        const ITE = 1 << 6;
        const FRI = (0xFF) << 8;
    }
}

static FAULT_EVENT_REGS: Once<SpinLock<FaultEventRegisters, LocalIrqDisabled>> = Once::new();

pub(super) unsafe fn init(base_register_vaddr: NonNull<u8>) {
    FAULT_EVENT_REGS
        .call_once(|| SpinLock::new(unsafe { FaultEventRegisters::new(base_register_vaddr) }));
}

fn iommu_fault_handler(_frame: &TrapFrame) {
    let mut fault_event_regs = FAULT_EVENT_REGS.get().unwrap().lock();

    primary_fault_handler(&mut fault_event_regs);

    let fault_status = fault_event_regs.status();
    if fault_status.intersects(FaultStatus::IQE | FaultStatus::ICE | FaultStatus::ITE) {
        panic!(
            "Catch IOMMU invalidation error. Fault status: {:x?}",
            fault_status
        );
    }
}

fn primary_fault_handler(fault_event_regs: &mut FaultEventRegisters) {
    let mut fault_status = fault_event_regs.status();
    if !fault_status.contains(FaultStatus::PPF) {
        return;
    }

    let start_index = ((fault_event_regs.status().bits() & FaultStatus::FRI.bits()) >> 8) as usize;
    let mut fault_iter = fault_event_regs.recordings.iter_mut();
    fault_iter.advance_by(start_index).unwrap();
    for raw_recording in fault_iter {
        let raw_recording = raw_recording.as_mut_ptr();
        let mut recording = FaultRecording(raw_recording.read());
        if !recording.is_fault() {
            break;
        }

        error!(
            "caught page fault, doing nothing. recording: {:x?}",
            recording
        );

        recording.clear_fault();
        raw_recording.write(recording.0);
    }

    if fault_status.contains(FaultStatus::PFO) {
        info!("Primary fault overflow detected.");
        fault_status.remove(FaultStatus::PFO);
        fault_event_regs
            .status
            .as_mut_ptr()
            .write(fault_status.bits());
    }
}
