use core::fmt::Debug;

use bitflags::bitflags;
use id_alloc::IdAlloc;
use int_to_c_enum::TryFromInt;

use super::IrtEntryHandle;
use crate::{
    mm::{FrameAllocOptions, HasPaddr, PAGE_SIZE, Segment, io::util::HasVmReaderWriter},
    sync::{LocalIrqDisabled, SpinLock},
};

#[expect(dead_code)]
#[derive(Debug)]
enum ExtendedInterruptMode {
    XApic,
    X2Apic,
}

struct IntRemappingMeta;
crate::impl_untyped_frame_meta_for!(IntRemappingMeta);

pub(in crate::arch::iommu) struct IntRemappingTable {
    num_entries: u16,
    extended_interrupt_mode: ExtendedInterruptMode,
    segment: Segment<IntRemappingMeta>,
    modification_lock: SpinLock<(), LocalIrqDisabled>,
    allocator: SpinLock<IdAlloc, LocalIrqDisabled>,
}

impl IntRemappingTable {
    pub(super) fn alloc(&'static self) -> Option<IrtEntryHandle> {
        let index = self.allocator.lock().alloc()?;
        Some(IrtEntryHandle {
            index: index as u16,
            table: self,
        })
    }

    pub(super) fn new() -> Self {
        const NUM_PAGES: usize = 1;

        let segment = FrameAllocOptions::new()
            .alloc_segment_with(NUM_PAGES, |_| IntRemappingMeta)
            .unwrap();
        let num_entries = (NUM_PAGES * PAGE_SIZE / size_of::<u128>())
            .try_into()
            .unwrap();

        Self {
            num_entries,
            extended_interrupt_mode: ExtendedInterruptMode::X2Apic,
            segment,
            modification_lock: SpinLock::new(()),
            allocator: SpinLock::new(IdAlloc::with_capacity(num_entries as usize)),
        }
    }

    pub(super) fn set_entry(&self, index: u16, entry: IrtEntry) {
        let _guard = self.modification_lock.lock();

        let [lower, upper] = entry.as_raw_u64();

        let offset = (index as usize) * size_of::<u128>();
        self.segment
            .writer()
            .skip(offset)
            .write_once(&0u64)
            .unwrap();
        self.segment
            .writer()
            .skip(offset + size_of::<u64>())
            .write_once(&upper)
            .unwrap();
        self.segment
            .writer()
            .skip(offset)
            .write_once(&lower)
            .unwrap();
    }

    pub(crate) fn encode(&self) -> u64 {
        let mut encoded = self.segment.paddr() as u64;

        encoded |= match self.extended_interrupt_mode {
            ExtendedInterruptMode::XApic => 0,
            ExtendedInterruptMode::X2Apic => 1 << 11,
        };

        encoded |= {
            assert!(self.num_entries.is_power_of_two());
            let size = self.num_entries.trailing_zeros().checked_sub(1).unwrap();
            assert!(size <= 15);
            size as u64
        };

        encoded
    }
}

#[repr(u32)]
#[derive(Debug, TryFromInt)]
pub(super) enum SourceValidationType {
    Disable = 0b00,
    RequesterId = 0b01,
    RequesterBus = 0b10,
    Reserved = 0b11,
}

#[repr(u32)]
#[derive(Debug, TryFromInt)]
pub(super) enum SourceIdQualifier {
    All = 0b00,
    IgnoreThirdLeast = 0b01,
    IgnoreSecondThirdLeast = 0b10,
    IgnoreLeastThree = 0b11,
}

#[expect(dead_code)]
#[repr(u32)]
#[derive(Debug, TryFromInt)]
enum DeliveryMode {
    FixedMode = 0b000,
    LowestPriority = 0b001,
    SystemManagementInterrupt = 0b010,
    NonMaskableInterrupt = 0b100,
    Init = 0b101,
    ExInt = 0b111,
}

pub(super) struct IrtEntry(u128);

impl IrtEntry {
    pub(super) fn new_enabled(vector: u32) -> Self {
        Self(0b11 | ((vector as u128) << 16))
    }

    fn as_raw_u64(&self) -> [u64; 2] {
        [self.0 as u64, (self.0 >> 64) as u64]
    }

    pub(super) fn source_validation_type(&self) -> SourceValidationType {
        const SVT_MASK: u128 = 0x3 << 82;
        SourceValidationType::try_from(((self.0 & SVT_MASK) >> 82) as u32).unwrap()
    }

    pub(super) fn source_id_qualifier(&self) -> SourceIdQualifier {
        const SQ_MASK: u128 = 0x3 << 82;
        SourceIdQualifier::try_from(((self.0 & SQ_MASK) >> 82) as u32).unwrap()
    }

    pub(super) const fn source_identifier(&self) -> u32 {
        const SID_MASK: u128 = 0xFFFF << 64;
        ((self.0 & SID_MASK) >> 64) as u32
    }

    pub(super) const fn destination_id(&self) -> u32 {
        const DST_MASK: u128 = 0xFFFF_FFFF << 32;
        ((self.0 & DST_MASK) >> 32) as u32
    }

    pub(super) const fn vector(&self) -> u8 {
        const VECTOR_MASK: u128 = 0xFF << 16;
        ((self.0 & VECTOR_MASK) >> 16) as u8
    }

    pub(super) const fn flags(&self) -> IrtEntryFlags {
        IrtEntryFlags::from_bits_truncate((self.0 & 0xFFFF_FFFF) as u32)
    }
}

impl Debug for IrtEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IrtEntry")
            .field("flags", &self.flags())
            .field("destination_id", &self.destination_id())
            .field("vector", &self.vector())
            .field("source_identifier", &self.source_identifier())
            .field("source_id_qualifier", &self.source_id_qualifier())
            .field("source_validation_type", &self.source_validation_type())
            .field("raw", &self.0)
            .finish()
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct IrtEntryFlags: u32 {
        const P   = 1 << 0;
        const FPD = 1 << 1;
        const DM  = 1 << 2;
        const RH  = 1 << 3;
        const TM  = 1 << 4;
        const IM  = 1 << 15;
    }
}
