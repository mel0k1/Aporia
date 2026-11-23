use core::{marker::PhantomData, ops::Range};

use crate::mm::{
    Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce,
    page_prop::{CachePolicy, PageFlags, PageTableFlags, PrivilegedPageFlags as PrivFlags},
    page_table::{PageTableConfig, PteScalar, PteTrait},
};

#[derive(Clone, Debug)]
pub(crate) struct IommuPtConfig {}

unsafe impl PageTableConfig for IommuPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = PtItem;
    type ItemRef<'a> = PtItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        (item.0, item.1, item.2)
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        (paddr, level, prop)
    }

    unsafe fn item_ref_from_raw<'a>(
        _paddr: Paddr,
        _level: PagingLevel,
        _prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        PhantomData
    }
}

pub(crate) type PtItem = (Paddr, PagingLevel, PageProperty);
pub(crate) type PtItemRef<'a> = PhantomData<&'a ()>;

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 3;
    const ADDRESS_WIDTH: usize = 39;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct PteFlags: usize {
        const SNOOP      = 1 << 11;

        const IGN2       = 1 << 9;
        const IGN1       = 1 << 8;

        const LAST_PAGE  = 1 << 7;

        const IGNORE_PAT = 1 << 6;

        const EMT        = 7 << 3;

        const WRITABLE   = 1 << 1;

        const READABLE   = 1 << 0;

    }
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
    const PHYS_MASK: usize = 0xffff_ffff_f000;

    fn is_present(&self) -> bool {
        self.0 & (PteFlags::READABLE | PteFlags::LAST_PAGE).bits() != 0
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::IGN2, PageFlags::AVAIL2);

        let priv_flags = parse_flags!(self.0, PteFlags::IGN1, PrivFlags::AVAIL1);

        let cache = if self.0 & PteFlags::SNOOP.bits() != 0 {
            CachePolicy::Writeback
        } else {
            CachePolicy::Uncacheable
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let bits = PageTableFlags::empty().bits() as usize
            | parse_flags!(self.0, PteFlags::IGN1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::IGN2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::LAST_PAGE.bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::AVAIL1, PteFlags::IGN1)
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::IGN2);

        if prop.cache != CachePolicy::Uncacheable {
            flags |= PteFlags::SNOOP.bits();
        }

        Self(paddr & Self::PHYS_MASK | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::READABLE.bits()
            | PteFlags::WRITABLE.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::IGN1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::IGN2);
        Self(paddr & Self::PHYS_MASK | flags)
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

        let paddr = self.0 & Self::PHYS_MASK;
        if self.is_last(level) {
            PteScalar::Mapped(paddr, self.prop())
        } else {
            PteScalar::PageTable(paddr, self.pt_flags())
        }
    }
}
