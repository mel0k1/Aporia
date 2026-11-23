use core::{arch::asm, ops::Range};

pub(crate) use util::{
    __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
};

use crate::mm::{
    PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
    page_prop::{
        CachePolicy, PageFlags, PageProperty, PageTableFlags, PrivilegedPageFlags as PrivFlags,
    },
    page_table::{PteScalar, PteTrait},
};

mod util;

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 4;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct PteFlags: usize {
        const VALID           = 1 << 0;
        const NON_HUGE        = 1 << 1;
        const USER            = 1 << 6;
        const NO_WRITE        = 1 << 7;
        const ACCESSED        = 1 << 10;
        const NON_GLOBAL      = 1 << 11;

        const DIRTY           = 1 << 51;
        const NO_EXECUTE_KERN = 1 << 53;
        const NO_EXECUTE_USER = 1 << 54;

        const HIGH_IGN1       = 1 << 55;
        const HIGH_IGN2       = 1 << 56;

        const ATTR_DEVICE     = 1 << 2;
        const SH_INNER        = 3 << 8;
    }
}

const SHARED_ASID: usize = 0;

const ASID_SHIFT: u32 = 48;

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    unsafe {
        asm!(
            "dsb nshst",
            "tlbi vaae1, {vpn}",
            "dsb nsh",
            "isb",
            vpn = in(reg) vaddr_to_vpn(vaddr),
        );
    }
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    unsafe {
        asm!("dsb nshst");
        for vaddr in range.clone().step_by(PAGE_SIZE) {
            asm!("tlbi vaae1, {vpn}", vpn = in(reg) vaddr_to_vpn(vaddr));
        }
        asm!("dsb nsh", "isb");
    }
}

fn vaddr_to_vpn(vaddr: Vaddr) -> usize {
    (vaddr >> 12) & ((1 << 44) - 1)
}

pub(crate) fn tlb_flush_all_excluding_global() {
    unsafe {
        asm!("dsb nshst", "tlbi aside1, {asid}", "dsb nsh", "isb", asid = in(reg) SHARED_ASID << ASID_SHIFT)
    };
}

pub(crate) fn tlb_flush_all_including_global() {
    unsafe { asm!("dsb nshst", "tlbi vmalle1", "dsb nsh", "isb") };
}

pub(crate) fn can_sync_dma() -> bool {
    true
}

pub(crate) unsafe fn sync_dma_range(mut range: Range<Vaddr>, is_from_device: bool) {
    use core::sync::atomic::{AtomicUsize, Ordering};

    static CACHE_LINE_SIZE: AtomicUsize = AtomicUsize::new(0);

    let mut cache_line_size = CACHE_LINE_SIZE.load(Ordering::Relaxed);
    if cache_line_size == 0 {
        let dmin_line = {
            let ctr: usize;
            unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr) };
            (ctr >> 16) & 0xf
        };
        cache_line_size = 4 << dmin_line;
        assert!(cache_line_size <= PAGE_SIZE);
        CACHE_LINE_SIZE.store(cache_line_size, Ordering::Relaxed);
    }

    range.start &= !(cache_line_size - 1);

    for vaddr in range.step_by(cache_line_size) {
        unsafe {
            if is_from_device {
                asm!("dc ivac, {}", in(reg) vaddr, options(nostack, preserves_flags));
            } else {
                asm!("dc cvac, {}", in(reg) vaddr, options(nostack, preserves_flags));
            }
        }
    }
}

pub(crate) unsafe fn activate_page_table(root_paddr: Paddr) {
    debug_assert_eq!(root_paddr >> ASID_SHIFT, 0);
    let ttbr = root_paddr | (SHARED_ASID << ASID_SHIFT);

    unsafe {
        asm!(
            "msr ttbr0_el1, {ttbr}",
            "msr ttbr1_el1, {ttbr}",
            "isb",
            ttbr = in(reg) ttbr,
        );
    }

    tlb_flush_all_excluding_global();
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    let ttbr: usize;
    unsafe {
        asm!(
            "mrs {ttbr}, ttbr0_el1",
            ttbr = out(reg) ttbr,
        );
    }

    debug_assert_eq!(ttbr >> ASID_SHIFT, SHARED_ASID);
    ttbr - (SHARED_ASID << ASID_SHIFT)
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
    const PHYS_ADDR_MASK: usize = 0x0000_FFFF_FFFF_F000;

    fn is_present(&self) -> bool {
        if self.0 & PteFlags::VALID.bits() != 0 {
            true
        } else if self.0 & PteFlags::SH_INNER.bits() != 0 {
            true
        } else {
            false
        }
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1 || self.0 & PteFlags::NON_HUGE.bits() == 0
    }

    fn paddr(&self) -> Paddr {
        self.0 & Self::PHYS_ADDR_MASK
    }

    fn prop(&self) -> PageProperty {
        let mut flags = parse_flags!(self.0, PteFlags::VALID, PageFlags::R)
            | parse_flags!(!self.0, PteFlags::NO_WRITE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::ACCESSED, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageFlags::AVAIL2);
        if self.0 & PteFlags::USER.bits() != 0 {
            flags |= parse_flags!(!self.0, PteFlags::NO_EXECUTE_USER, PageFlags::X);
        } else {
            flags |= parse_flags!(!self.0, PteFlags::NO_EXECUTE_KERN, PageFlags::X);
        }

        let priv_flags = parse_flags!(self.0, PteFlags::USER, PrivFlags::USER)
            | parse_flags!(!self.0, PteFlags::NON_GLOBAL, PrivFlags::GLOBAL)
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PrivFlags::AVAIL1);

        let cache = if self.0 & PteFlags::ATTR_DEVICE.bits() != 0 {
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
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::ACCESSED.bits();
        if level == 1 {
            flags |= PteFlags::NON_HUGE.bits();
        }

        flags |= parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::VALID)
            | parse_flags!(!prop.flags.bits(), PageFlags::W, PteFlags::NO_WRITE)
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::ACCESSED)
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::USER, PteFlags::USER)
            | parse_flags!(
                !prop.priv_flags.bits(),
                PrivFlags::GLOBAL,
                PteFlags::NON_GLOBAL
            )
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1,
                PteFlags::HIGH_IGN1
            )
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::HIGH_IGN2);
        if prop.priv_flags.contains(PrivFlags::USER) {
            flags |= PteFlags::NO_EXECUTE_KERN.bits()
                | parse_flags!(!prop.flags.bits(), PageFlags::X, PteFlags::NO_EXECUTE_USER);
        } else {
            flags |= PteFlags::NO_EXECUTE_USER.bits()
                | parse_flags!(!prop.flags.bits(), PageFlags::X, PteFlags::NO_EXECUTE_KERN);
        }

        flags |= PteFlags::SH_INNER.bits();
        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                flags |= PteFlags::ATTR_DEVICE.bits();
            }
            _ => panic!("unsupported cache policy"),
        }

        debug_assert_eq!(
            paddr & !Self::PHYS_ADDR_MASK,
            0,
            "page physical address contains invalid bits"
        );
        Self(paddr | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::VALID.bits()
            | PteFlags::NON_HUGE.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::HIGH_IGN1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::HIGH_IGN2);

        debug_assert_eq!(
            paddr & !Self::PHYS_ADDR_MASK,
            0,
            "page table physical address contains invalid bits"
        );
        Self(paddr | flags)
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
        if !self.is_present() {
            return PteScalar::Absent;
        }

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}
