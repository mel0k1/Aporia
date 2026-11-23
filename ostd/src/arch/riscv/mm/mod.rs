use core::ops::Range;

use spin::Once;
pub(crate) use util::{
    __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
};

use crate::{
    arch::{
        boot::DEVICE_TREE,
        cpu::extension::{IsaExtensions, has_extensions},
    },
    mm::{
        PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
        page_prop::{
            CachePolicy, PageFlags, PageProperty, PageTableFlags, PrivilegedPageFlags as PrivFlags,
        },
        page_table::{PteScalar, PteTrait},
    },
};

mod util;

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

#[cfg(not(feature = "riscv_sv39_mode"))]
impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 4;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

#[cfg(feature = "riscv_sv39_mode")]
impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 3;
    const ADDRESS_WIDTH: usize = 39;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 2;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct PteFlags: usize {
        const VALID      = 1 << 0;
        const READABLE   = 1 << 1;
        const WRITABLE   = 1 << 2;
        const EXECUTABLE = 1 << 3;
        const USER       = 1 << 4;
        const GLOBAL     = 1 << 5;
        const ACCESSED   = 1 << 6;
        const DIRTY      = 1 << 7;

        const RSV1       = 1 << 8;
        const RSV2       = 1 << 9;

        const PBMT_NC    = 1 << 61;
        const PBMT_IO    = 1 << 62;
        const NAPOT      = 1 << 63;
    }
}

const SHARED_ASID: usize = 0;

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    unsafe { core::arch::riscv64::sfence_vma_vaddr(vaddr) };
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

pub(crate) fn tlb_flush_all_excluding_global() {
    unsafe { core::arch::riscv64::sfence_vma_asid(SHARED_ASID) };
}

pub(crate) fn tlb_flush_all_including_global() {
    unsafe { core::arch::riscv64::sfence_vma_all() };
}

pub(crate) fn can_sync_dma() -> bool {
    has_extensions(IsaExtensions::ZICBOM)
}

pub(crate) unsafe fn sync_dma_range(mut range: Range<Vaddr>, is_from_device: bool) {
    use core::arch::asm;

    debug_assert!(can_sync_dma());

    static CMO_MANAGEMENT_BLOCK_SIZE: Once<usize> = Once::new();
    let cmo_management_block_size = *CMO_MANAGEMENT_BLOCK_SIZE.call_once(|| {
        let block_size = DEVICE_TREE
            .get()
            .unwrap()
            .cpus()
            .find(|cpu| cpu.property("mmu-type").is_some())
            .expect("Failed to find an application CPU node in device tree")
            .property("riscv,cbom-block-size")
            .expect("Failed to find `riscv,cbom-block-size` property of the CPU node")
            .as_usize()
            .expect("Failed to parse `riscv,cbom-block-size` property of the CPU node");
        assert!(block_size.is_power_of_two());
        assert!(block_size <= PAGE_SIZE);
        block_size
    });

    range.start &= !(cmo_management_block_size - 1);

    for addr in range.step_by(cmo_management_block_size) {
        unsafe {
            if is_from_device {
                asm!("cbo.inval ({})", in(reg) addr, options(nostack, preserves_flags));
            } else {
                asm!("cbo.clean ({})", in(reg) addr, options(nostack, preserves_flags));
            }
        }
    }
}

pub(crate) unsafe fn activate_page_table(root_paddr: Paddr) {
    assert!(root_paddr.is_multiple_of(PagingConsts::BASE_PAGE_SIZE));
    let ppn = root_paddr >> 12;

    #[cfg(not(feature = "riscv_sv39_mode"))]
    let mode = riscv::register::satp::Mode::Sv48;
    #[cfg(feature = "riscv_sv39_mode")]
    let mode = riscv::register::satp::Mode::Sv39;

    unsafe { riscv::register::satp::set(mode, SHARED_ASID, ppn) };

    tlb_flush_all_excluding_global();
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    riscv::register::satp::read().ppn() << 12
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct PageTableEntry(usize);

macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = 0x003f_ffff_ffff_fc00;

    fn new_without_flags(paddr: Paddr) -> Self {
        assert_eq!(paddr & !Self::PHYS_ADDR_MASK, 0);
        Self(paddr >> 12 << 10)
    }

    fn paddr(&self) -> Paddr {
        (self.0 & Self::PHYS_ADDR_MASK) >> 10 << 12
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        let rwx = PteFlags::READABLE | PteFlags::WRITABLE | PteFlags::EXECUTABLE;
        level == 1 || (self.0 & rwx.bits()) != 0
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::EXECUTABLE, PageFlags::X)
            | parse_flags!(self.0, PteFlags::ACCESSED, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::RSV2, PageFlags::AVAIL2);

        let priv_flags = parse_flags!(self.0, PteFlags::USER, PrivFlags::USER)
            | parse_flags!(self.0, PteFlags::GLOBAL, PrivFlags::GLOBAL)
            | parse_flags!(self.0, PteFlags::RSV1, PrivFlags::AVAIL1);

        let cache = if self.0 & PteFlags::PBMT_IO.bits() != 0 {
            CachePolicy::Uncacheable
        } else {
            CachePolicy::Writeback
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let bits = PageTableFlags::empty().bits() as usize
            | parse_flags!(self.0, PteFlags::RSV1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::RSV2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::VALID.bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::X, PteFlags::EXECUTABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::ACCESSED)
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::USER, PteFlags::USER)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::GLOBAL, PteFlags::GLOBAL)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::AVAIL1, PteFlags::RSV1)
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::RSV2);

        if has_extensions(IsaExtensions::SVADE) {
            flags |= (PteFlags::ACCESSED | PteFlags::DIRTY).bits();
        }

        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                if has_extensions(IsaExtensions::SVPBMT) {
                    flags |= PteFlags::PBMT_IO.bits()
                }
            }
            _ => panic!("unsupported cache policy"),
        }

        let res = Self::new_without_flags(paddr);
        Self(res.0 | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::VALID.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::RSV1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::RSV2);

        let res = Self::new_without_flags(paddr);
        Self(res.0 | flags)
    }
}

impl PodOnce for PageTableEntry {}

unsafe impl PteTrait for PageTableEntry {
    fn from_repr(repr: &PteScalar, level: PagingLevel) -> Self {
        match repr {
            PteScalar::Absent => PageTableEntry(0),
            PteScalar::PageTable(paddr, flags) => Self::new_pt(*paddr, *flags),
            PteScalar::Mapped(paddr, prop) => Self::new_page(*paddr, level, *prop),
        }
    }

    fn to_repr(&self, level: PagingLevel) -> PteScalar {
        if self.0 & PteFlags::VALID.bits() == 0 {
            return PteScalar::Absent;
        }

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}
