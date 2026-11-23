#![cfg_attr(target_arch = "loongarch64", expect(unused_imports))]

pub(crate) mod kvirt_area;

use core::ops::Range;

use spin::Once;

#[cfg(ktest)]
mod test;

use super::{
    Frame, HasSize, Paddr, PagingConstsTrait, Vaddr,
    frame::{
        Segment,
        meta::{AnyFrameMeta, MetaPageMeta, mapping},
    },
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    page_table::{PageTable, PageTableConfig, largest_pages, max_page_level},
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    boot::memory_region::MemoryRegionType,
    const_assert, info,
    mm::{HasPaddr, PAGE_SIZE, PagingLevel, frame::FrameRef},
    task::disable_preempt,
};

const_assert!(PagingConsts::ADDRESS_WIDTH >= 39);
const ADDR_WIDTH_SHIFT: usize = PagingConsts::ADDRESS_WIDTH - 39;

#[cfg(not(target_arch = "loongarch64"))]
pub(super) const KERNEL_BASE_VADDR: Vaddr = 0xffff_ffc0_0000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "loongarch64")]
pub(super) const KERNEL_BASE_VADDR: Vaddr = 0x9000_0000_0000_0000;
pub(super) const KERNEL_END_VADDR: Vaddr = 0xffff_ffff_ffff_0000;

pub const MAX_USERSPACE_VADDR: Vaddr = (0x0000_0040_0000_0000 << ADDR_WIDTH_SHIFT) - PAGE_SIZE;

pub const KERNEL_VADDR_RANGE: Range<Vaddr> = KERNEL_BASE_VADDR..KERNEL_END_VADDR;

pub(crate) fn kernel_loaded_offset() -> usize {
    #[cfg(target_arch = "riscv64")]
    {
        unsafe extern "C" {
            static __kernel_loaded_offset: usize;
        }
        unsafe { __kernel_loaded_offset }
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        KERNEL_CODE_BASE_VADDR
    }
}

#[cfg(target_arch = "x86_64")]
const KERNEL_CODE_BASE_VADDR: usize = 0xffff_ffff_8000_0000;
#[cfg(target_arch = "aarch64")]
const KERNEL_CODE_BASE_VADDR: usize = 0xffff_ffff_0000_0000;
#[cfg(target_arch = "loongarch64")]
const KERNEL_CODE_BASE_VADDR: usize = 0x9000_0000_0000_0000;

const FRAME_METADATA_CAP_VADDR: Vaddr = 0xffff_fff0_8000_0000 << ADDR_WIDTH_SHIFT;
const FRAME_METADATA_BASE_VADDR: Vaddr = 0xffff_fff0_0000_0000 << ADDR_WIDTH_SHIFT;
pub(super) const FRAME_METADATA_RANGE: Range<Vaddr> =
    FRAME_METADATA_BASE_VADDR..FRAME_METADATA_CAP_VADDR;

const VMALLOC_BASE_VADDR: Vaddr = 0xffff_ffe0_0000_0000 << ADDR_WIDTH_SHIFT;
pub(super) const VMALLOC_VADDR_RANGE: Range<Vaddr> = VMALLOC_BASE_VADDR..FRAME_METADATA_BASE_VADDR;

#[cfg(not(target_arch = "loongarch64"))]
pub(crate) const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_ffc0_0000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "loongarch64")]
pub(crate) const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0x9000_0000_0000_0000;
pub(crate) const LINEAR_MAPPING_VADDR_RANGE: Range<Vaddr> =
    LINEAR_MAPPING_BASE_VADDR..VMALLOC_BASE_VADDR;

pub(crate) fn paddr_to_vaddr(pa: Paddr) -> usize {
    debug_assert!(pa < VMALLOC_BASE_VADDR - LINEAR_MAPPING_BASE_VADDR);
    pa + LINEAR_MAPPING_BASE_VADDR
}

pub(super) static KERNEL_PAGE_TABLE: Once<PageTable<KernelPtConfig>> = Once::new();

#[derive(Clone, Debug)]
pub(super) struct KernelPtConfig {}

unsafe impl PageTableConfig for KernelPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 256..512;
    const TOP_LEVEL_CAN_UNMAP: bool = false;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = MappedItem;
    type ItemRef<'a> = MappedItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        match *item {
            MappedItem::Tracked(ref frame, mut prop) => {
                debug_assert!(!prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1));
                prop.priv_flags |= PrivilegedPageFlags::AVAIL1;
                let level = frame.map_level();
                let paddr = frame.paddr();
                (paddr, level, prop)
            }
            MappedItem::Untracked(ref pa, ref level, mut prop) => {
                debug_assert!(!prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1));
                prop.priv_flags -= PrivilegedPageFlags::AVAIL1;
                (*pa, *level, prop)
            }
        }
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            debug_assert_eq!(level, 1);
            let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
            MappedItem::Tracked(frame, prop)
        } else {
            MappedItem::Untracked(paddr, level, prop)
        }
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            debug_assert_eq!(level, 1);
            let frame = unsafe { FrameRef::<dyn AnyFrameMeta>::borrow_paddr(paddr) };
            MappedItemRef::Tracked(frame, prop)
        } else {
            MappedItemRef::Untracked(paddr, level, prop)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum MappedItem {
    Tracked(Frame<dyn AnyFrameMeta>, PageProperty),
    Untracked(Paddr, PagingLevel, PageProperty),
}

#[derive(Debug)]
pub(crate) enum MappedItemRef<'a> {
    #[cfg_attr(not(ktest), expect(dead_code))]
    Tracked(FrameRef<'a, dyn AnyFrameMeta>, PageProperty),
    #[cfg_attr(not(ktest), expect(dead_code))]
    Untracked(Paddr, PagingLevel, PageProperty),
}

pub(crate) fn init_kernel_page_table(meta_pages: Segment<MetaPageMeta>) {
    info!("Initializing the kernel page table");

    let kpt = PageTable::<KernelPtConfig>::new_kernel_page_table();
    let preempt_guard = disable_preempt();

    #[cfg(not(target_arch = "loongarch64"))]
    {
        let max_paddr = crate::mm::frame::max_paddr();
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + max_paddr;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let min_level = max_page_level::<KernelPtConfig>(from.len());
        let mut cursor = kpt
            .cursor_mut_with_min_level(&preempt_guard, &from, min_level)
            .unwrap();
        for (pa, level) in largest_pages::<KernelPtConfig>(from.start, 0, max_paddr) {
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) };
        }
    }

    {
        let start_va = mapping::frame_to_meta::<PagingConsts>(crate::mm::frame::min_paddr());
        let from = start_va..start_va + meta_pages.size();
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let min_level = max_page_level::<KernelPtConfig>(from.len());
        let mut cursor = kpt
            .cursor_mut_with_min_level(&preempt_guard, &from, min_level)
            .unwrap();
        let pa_range = meta_pages.into_raw();
        for (pa, level) in
            largest_pages::<KernelPtConfig>(from.start, pa_range.start, pa_range.len())
        {
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) };
        }
    }

    #[cfg(not(target_arch = "loongarch64"))]
    {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        let region = regions
            .iter()
            .find(|r| r.typ() == MemoryRegionType::Kernel)
            .unwrap();
        let offset = kernel_loaded_offset();
        let from = region.base() + offset..region.end() + offset;
        let prop = PageProperty {
            flags: PageFlags::RWX,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let min_level = max_page_level::<KernelPtConfig>(from.len());
        let mut cursor = kpt
            .cursor_mut_with_min_level(&preempt_guard, &from, min_level)
            .unwrap();
        for (pa, level) in largest_pages::<KernelPtConfig>(from.start, region.base(), from.len()) {
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) };
        }
    }

    KERNEL_PAGE_TABLE.call_once(|| kpt);
}

pub(crate) unsafe fn activate_kernel_page_table() {
    let kpt = KERNEL_PAGE_TABLE
        .get()
        .expect("The kernel page table is not initialized yet");
    unsafe {
        kpt.first_activate_unchecked();
        crate::arch::mm::tlb_flush_all_including_global();
    }
}
