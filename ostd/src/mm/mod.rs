#![cfg_attr(
    any(
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "aarch64"
    ),
    expect(unused_imports)
)]

pub mod dma;
pub mod fault;
pub mod frame;
pub mod heap;
pub mod io;
pub(crate) mod kspace;
pub(crate) mod mem_obj;
pub(crate) mod page_prop;
pub(crate) mod page_table;
pub mod tlb;
pub mod vm_space;

#[cfg(ktest)]
mod test;

use core::fmt::Debug;

pub use self::{
    frame::{
        Frame,
        allocator::FrameAllocOptions,
        segment::{Segment, USegment},
        unique::UniqueFrame,
        untyped::{AnyUFrameMeta, UFrame},
    },
    io::{
        Fallible, FallibleVmRead, FallibleVmWrite, Infallible, PodAtomic, PodOnce, VmIo, VmIoFill,
        VmIoOnce, VmReader, VmWriter,
    },
    kspace::{KERNEL_VADDR_RANGE, MAX_USERSPACE_VADDR},
    mem_obj::{HasDaddr, HasPaddr, HasPaddrRange, HasSize, Split},
    page_prop::{CachePolicy, PageFlags, PageProperty},
    vm_space::VmSpace,
};
pub(crate) use self::{
    kspace::paddr_to_vaddr,
    page_prop::{PageTableFlags, PrivilegedPageFlags},
    page_table::PageTable,
};
use crate::arch::mm::PagingConsts;

pub type Vaddr = usize;

pub type Paddr = usize;

pub type Daddr = usize;

pub fn is_in_user_space(vaddr: Vaddr, len: usize) -> bool {
    vaddr.saturating_add(len) <= MAX_USERSPACE_VADDR
}

pub type PagingLevel = u8;

pub(crate) trait PagingConstsTrait: Clone + Debug + Send + Sync + 'static {
    const BASE_PAGE_SIZE: usize;

    const NR_LEVELS: PagingLevel;

    const HIGHEST_TRANSLATION_LEVEL: PagingLevel;

    const PTE_SIZE: usize;

    const ADDRESS_WIDTH: usize;

    const VA_SIGN_EXT: bool;
}

pub const PAGE_SIZE: usize = page_size::<PagingConsts>(1);

pub(crate) const fn page_size<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    C::BASE_PAGE_SIZE << (nr_subpage_per_huge::<C>().ilog2() as usize * (level as usize - 1))
}

pub(crate) const fn nr_subpage_per_huge<C: PagingConstsTrait>() -> usize {
    C::BASE_PAGE_SIZE / C::PTE_SIZE
}

#[expect(dead_code)]
pub(crate) const fn nr_base_per_page<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    page_size::<C>(level) / C::BASE_PAGE_SIZE
}
