use core::{ops::Range, ptr::NonNull};

use linux_boot_params::{BootParams, EfiInfo};
use tdx_guest::unaccepted_memory::{EfiUnacceptedMemory, EfiUnacceptedMemoryBuilder};
use uefi::{
    boot::AllocateType,
    mem::memory_map::{MemoryMap, MemoryType},
};

use crate::x86::amd64_efi::{alloc, efi};

pub(super) fn prepare_unaccepted_memory(
    boot_params: &mut BootParams,
) -> Option<EfiUnacceptedMemoryBuilder> {
    if !tdx_guest::is_tdx_guest_early() {
        return None;
    }

    boot_params.efi_info.efi_loader_signature = EfiInfo::TDX_LOADER_SIGNATURE;
    boot_params.efi_info.efi_systab = 0;
    boot_params.efi_info.efi_systab_hi = 0;

    let coverage = {
        let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA)
            .expect("failed to fetch pre-exit memory map for unaccepted bitmap setup");
        let mut ranges = iter_unaccepted_memory(&pre_exit_memory_map);

        let first_range = ranges.next()?;
        ranges.fold(first_range, |coverage, range| {
            coverage.start.min(range.start)..coverage.end.max(range.end)
        })
    };

    let builder = {
        let required_size = EfiUnacceptedMemory::required_size_for_range(coverage.clone())
            .expect("invalid unaccepted memory bounds");

        let allocation =
            alloc::alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), required_size);
        let allocation_ptr = NonNull::new(allocation.as_mut_ptr().cast()).unwrap();
        unsafe { EfiUnacceptedMemory::builder(allocation_ptr, allocation.len(), coverage) }
            .expect("failed to initialize unaccepted memory builder")
    };

    uefi::println!("[EFI stub] Passing unaccepted memory bitmap to the kernel");

    Some(builder)
}

pub(super) unsafe fn finish_unaccepted_memory<M: MemoryMap>(
    boot_params: &mut BootParams,
    builder: Option<EfiUnacceptedMemoryBuilder>,
    memory_map: &M,
) {
    let Some(mut builder) = builder else {
        return;
    };

    for range in iter_unaccepted_memory(memory_map) {
        unsafe { builder.register_range(range.start, range.end) }
            .expect("failed to register unaccepted memory range");
    }

    let table = builder.build();
    let table_addr = core::ptr::from_ref(table.as_ref().get_ref()).addr();
    let table_addr = table_addr
        .try_into()
        .expect("unaccepted memory table is above 4 GiB");
    boot_params.efi_info.efi_systab = table_addr;
}

fn iter_unaccepted_memory<M: MemoryMap>(memory_map: &M) -> impl Iterator<Item = Range<u64>> + '_ {
    memory_map.entries().filter_map(|entry| {
        if entry.ty != MemoryType::UNACCEPTED {
            return None;
        }

        let size = entry.page_count * efi::PAGE_SIZE;
        let end = entry.phys_start + size;
        Some(entry.phys_start..end)
    })
}
