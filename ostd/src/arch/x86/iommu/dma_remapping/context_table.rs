#![expect(dead_code)]

use alloc::collections::BTreeMap;

use super::second_stage::IommuPtConfig;
use crate::{
    arch::iommu::dma_remapping::PciDeviceLocation,
    debug,
    mm::{
        Daddr, Frame, FrameAllocOptions, HasPaddr, PAGE_SIZE, Paddr, PageFlags, PageTable, VmIo,
        page_prop::{CachePolicy, PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::PageTableError,
    },
    task::disable_preempt,
};

#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct RootEntry(u128);

impl RootEntry {
    const fn is_present(&self) -> bool {
        (self.0 & 0b1) != 0
    }

    const fn addr(&self) -> u64 {
        (self.0 & 0xFFFF_FFFF_FFFF_F000) as u64
    }
}

pub(in crate::arch::iommu) struct RootTable {
    root_frame: Frame<()>,
    context_tables: BTreeMap<Paddr, ContextTable>,
}

#[derive(Debug)]
pub(super) enum ContextTableError {
    InvalidDeviceId,
    ModificationError(PageTableError),
}

impl RootTable {
    pub(in crate::arch::iommu) fn root_paddr(&self) -> Paddr {
        self.root_frame.paddr()
    }

    pub(super) fn new() -> Self {
        Self {
            root_frame: FrameAllocOptions::new().alloc_frame().unwrap(),
            context_tables: BTreeMap::new(),
        }
    }

    pub(super) unsafe fn map(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
        paddr: Paddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        let context_table = self.get_or_create_context_table(device);
        unsafe { context_table.map(device, daddr, paddr)? };

        Ok(())
    }

    pub(super) fn unmap(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        let context_table = self.get_or_create_context_table(device);
        context_table.unmap(device, daddr)?;

        Ok(())
    }

    pub(super) fn specify_device_page_table(
        &mut self,
        device_id: PciDeviceLocation,
        page_table: PageTable<IommuPtConfig>,
    ) {
        let context_table = self.get_or_create_context_table(device_id);

        let bus_entry = context_table
            .entries_frame
            .read_val::<ContextEntry>(
                (device_id.device as usize * 8 + device_id.function as usize)
                    * size_of::<ContextEntry>(),
            )
            .unwrap();

        if bus_entry.is_present() {
            panic!("existing device page tables should not be overridden");
        }

        let address = page_table.root_paddr();
        context_table.page_tables.insert(address, page_table);
        let entry = ContextEntry(address as u128 | 1 | 0x1_0000_0000_0000_0000);
        context_table
            .entries_frame
            .write_val::<ContextEntry>(
                (device_id.device as usize * 8 + device_id.function as usize)
                    * size_of::<ContextEntry>(),
                &entry,
            )
            .unwrap();
    }

    fn get_or_create_context_table(&mut self, device_id: PciDeviceLocation) -> &mut ContextTable {
        let bus_entry = self
            .root_frame
            .read_val::<RootEntry>(device_id.bus as usize * size_of::<RootEntry>())
            .unwrap();

        if !bus_entry.is_present() {
            let table = ContextTable::new();
            let address = table.paddr();
            self.context_tables.insert(address, table);
            let entry = RootEntry(address as u128 | 1);
            self.root_frame
                .write_val::<RootEntry>(device_id.bus as usize * size_of::<RootEntry>(), &entry)
                .unwrap();
            self.context_tables.get_mut(&address).unwrap()
        } else {
            self.context_tables
                .get_mut(&(bus_entry.addr() as usize))
                .unwrap()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct ContextEntry(u128);

impl ContextEntry {
    pub(super) const fn domain_identifier(&self) -> u64 {
        ((self.0 & 0xFF_FF00_0000_0000_0000_0000) >> 72) as u64
    }

    pub(super) const fn address_width(&self) -> AddressWidth {
        let value = ((self.0 & 0x7_0000_0000_0000_0000) >> 64) as u64;
        match value {
            1 => AddressWidth::Level3PageTable,
            2 => AddressWidth::Level4PageTable,
            3 => AddressWidth::Level5PageTable,
            _ => AddressWidth::Reserved,
        }
    }

    pub(super) const fn second_stage_pointer(&self) -> u64 {
        (self.0 & 0xFFFF_FFFF_FFFF_F000) as u64
    }

    pub(super) const fn translation_type(&self) -> u64 {
        ((self.0 & 0b1100) >> 2) as u64
    }

    pub(super) const fn need_fault_process(&self) -> bool {
        (self.0 & 0b10) == 0
    }

    pub(super) const fn is_present(&self) -> bool {
        (self.0 & 0b1) != 0
    }
}

#[derive(Debug)]
pub(super) enum AddressWidth {
    Reserved,
    Level3PageTable,
    Level4PageTable,
    Level5PageTable,
}

pub(super) struct ContextTable {
    entries_frame: Frame<()>,
    page_tables: BTreeMap<Paddr, PageTable<IommuPtConfig>>,
}

impl ContextTable {
    fn new() -> Self {
        Self {
            entries_frame: FrameAllocOptions::new().alloc_frame().unwrap(),
            page_tables: BTreeMap::new(),
        }
    }

    fn paddr(&self) -> Paddr {
        self.entries_frame.paddr()
    }

    fn get_or_create_page_table(
        &mut self,
        device: PciDeviceLocation,
    ) -> &mut PageTable<IommuPtConfig> {
        let bus_entry = self
            .entries_frame
            .read_val::<ContextEntry>(
                (device.device as usize * 8 + device.function as usize) * size_of::<ContextEntry>(),
            )
            .unwrap();

        if !bus_entry.is_present() {
            let table = PageTable::<IommuPtConfig>::empty();
            let address = table.root_paddr();
            self.page_tables.insert(address, table);
            let entry = ContextEntry(address as u128 | 3 | 0x1_0000_0000_0000_0000);
            self.entries_frame
                .write_val::<ContextEntry>(
                    (device.device as usize * 8 + device.function as usize)
                        * size_of::<ContextEntry>(),
                    &entry,
                )
                .unwrap();
            self.page_tables.get_mut(&address).unwrap()
        } else {
            self.page_tables
                .get_mut(&(bus_entry.second_stage_pointer() as usize))
                .unwrap()
        }
    }

    unsafe fn map(
        &mut self,
        device: PciDeviceLocation,
        daddr: Daddr,
        paddr: Paddr,
    ) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        debug!(
            "Mapping Daddr: {:x?} to Paddr: {:x?} for device: {:x?}",
            daddr, paddr, device
        );

        let from = daddr..daddr + PAGE_SIZE;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Uncacheable,
            priv_flags: PrivFlags::empty(),
        };

        let pt = self.get_or_create_page_table(device);
        let preempt_guard = disable_preempt();
        let mut cursor = pt.cursor_mut(&preempt_guard, &from).unwrap();

        unsafe { cursor.map((paddr, 1, prop)) };

        Ok(())
    }

    fn unmap(&mut self, device: PciDeviceLocation, daddr: Daddr) -> Result<(), ContextTableError> {
        if device.device >= 32 || device.function >= 8 {
            return Err(ContextTableError::InvalidDeviceId);
        }

        debug!("Unmapping Daddr: {:x?} for device: {:x?}", daddr, device);

        let pt = self.get_or_create_page_table(device);
        let preempt_guard = disable_preempt();
        let mut cursor = pt
            .cursor_mut(&preempt_guard, &(daddr..daddr + PAGE_SIZE))
            .unwrap();

        let frag = unsafe { cursor.take_next(PAGE_SIZE) };
        debug_assert!(frag.is_some());

        Ok(())
    }
}
