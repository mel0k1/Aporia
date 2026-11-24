//! I/O memory utilities and port-I/O limit definitions.

use alloc::vec::Vec;

use align_ext::AlignExt;

use crate::{boot::memory_region::MemoryRegionType, io::IoMemAllocatorBuilder};

pub(crate) mod io_mem;

#[cfg(feature = "cvm_guest")]
pub(crate) mod cvm;

/// Initializes the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). The area from the top of low memory to 0xffff_ffff and the area after the
/// top of high memory are available MMIO areas.
///
/// This memory layout is documented in Intel's datasheets. The MMIO area is defined by specific
/// registers that are configured by the BIOS and locked down, preventing the OS from reconfiguring
/// them. For the details, one can read the "Processor Configuration Register Definitions and
/// Address Ranges" section in the "10th Generation Intel(R) Processor Families" datasheet
/// (<https://www.intel.com/content/dam/www/public/us/en/documents/datasheets/10th-gen-core-families-datasheet-vol-2-datasheet.pdf>).
/// However, note that these specifics may differ between CPU generations and those manufactured by
/// other vendors.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function must be called after the kernel page table is activated on
///    the bootstrapping processor.
pub(super) unsafe fn construct_io_mem_allocator_builder() -> IoMemAllocatorBuilder {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let mut ranges = Vec::with_capacity(2);

    let reserved_filter = regions.iter().filter(|r| {
        r.typ() != MemoryRegionType::Unknown
            && r.typ() != MemoryRegionType::Reserved
            && r.typ() != MemoryRegionType::Framebuffer
    });

    const LOW_MMIO_TOP: usize = 0x1_0000_0000;
    const LOW_MMIO_ALIGN: usize = 0x10_0000;
    let (lower_half_base, lower_half_len) = reserved_filter
        .clone()
        .filter(|r| r.base() < LOW_MMIO_TOP)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap();

    let mmio_start_addr = (lower_half_base + lower_half_len).align_up(LOW_MMIO_ALIGN);
    assert!(mmio_start_addr < LOW_MMIO_TOP);
    ranges.push(mmio_start_addr..LOW_MMIO_TOP);

    const HIGH_MMIO_TOP: usize = 0x10_0000_0000_0000;
    const HIGH_MMIO_ALIGN: usize = 0x10_0000;
    let (upper_half_base, upper_half_len) = reserved_filter
        .filter(|r| r.base() >= LOW_MMIO_TOP)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap_or((LOW_MMIO_TOP, 0));

    let mmio_start_addr = (upper_half_base + upper_half_len).align_up(HIGH_MMIO_ALIGN);
    assert!(mmio_start_addr < HIGH_MMIO_TOP);
    ranges.push(mmio_start_addr..HIGH_MMIO_TOP);

    unsafe { IoMemAllocatorBuilder::new(ranges) }
}

/// Port I/O definition reference: <https://bochs.sourceforge.io/techspec/PORTS.LST>.
pub(crate) const MAX_IO_PORT: u16 = u16::MAX;
