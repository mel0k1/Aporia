pub(crate) mod mapping {

    use super::MetaSlot;
    use crate::mm::{PAGE_SIZE, Paddr, PagingConstsTrait, Vaddr, kspace::FRAME_METADATA_RANGE};

    pub(crate) const fn frame_to_meta<C: PagingConstsTrait>(paddr: Paddr) -> Vaddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = paddr / PAGE_SIZE;
        base + offset * size_of::<MetaSlot>()
    }

    pub(crate) const fn meta_to_frame<C: PagingConstsTrait>(vaddr: Vaddr) -> Paddr {
        let base = FRAME_METADATA_RANGE.start;
        let offset = (vaddr - base) / size_of::<MetaSlot>();
        offset * PAGE_SIZE
    }
}

use core::{
    alloc::Layout,
    any::Any,
    cell::UnsafeCell,
    fmt::Debug,
    mem::{ManuallyDrop, MaybeUninit},
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
};

use align_ext::AlignExt;

use crate::{
    arch::mm::PagingConsts,
    boot::memory_region::{MemoryRegion, MemoryRegionType},
    const_assert, info,
    mm::{
        CachePolicy, Infallible, PAGE_SIZE, Paddr, PageFlags, PageProperty, PrivilegedPageFlags,
        Segment, Vaddr, VmReader,
        frame::allocator::{self, EarlyAllocatedFrameMeta},
        paddr_to_vaddr, page_size,
        page_table::boot_pt,
    },
    panic::abort,
    util::ops::range_difference,
};

pub const FRAME_METADATA_MAX_SIZE: usize = META_SLOT_SIZE
    - size_of::<AtomicU64>()
    - size_of::<FrameMetaVtablePtr>()
    - size_of::<AtomicU64>();
pub const FRAME_METADATA_MAX_ALIGN: usize = META_SLOT_SIZE;

const META_SLOT_SIZE: usize = 64;

#[repr(C)]
pub(in crate::mm) struct MetaSlot {
    storage: UnsafeCell<[u8; FRAME_METADATA_MAX_SIZE]>,
    pub(super) ref_count: AtomicU64,
    pub(super) vtable_ptr: UnsafeCell<MaybeUninit<FrameMetaVtablePtr>>,
    pub(super) in_list: AtomicU64,
}

pub(super) const REF_COUNT_UNUSED: u64 = u64::MAX;
pub(super) const REF_COUNT_UNIQUE: u64 = u64::MAX - 1;
pub(super) const REF_COUNT_MAX: u64 = i64::MAX as u64;

type FrameMetaVtablePtr = core::ptr::DynMetadata<dyn AnyFrameMeta>;

const_assert!(PAGE_SIZE.is_multiple_of(META_SLOT_SIZE));
const_assert!(size_of::<MetaSlot>() == META_SLOT_SIZE);

pub unsafe trait AnyFrameMeta: Any + Send + Sync {
    fn on_drop(&mut self, _reader: &mut VmReader<Infallible>) {}

    fn is_untyped(&self) -> bool {
        false
    }
}

#[macro_export]
macro_rules! check_frame_meta_layout {
    ($t:ty) => {
        $crate::const_assert!(size_of::<$t>() <= $crate::mm::frame::meta::FRAME_METADATA_MAX_SIZE);
        $crate::const_assert!(
            $crate::mm::frame::meta::FRAME_METADATA_MAX_ALIGN % align_of::<$t>() == 0
        );
    };
}

#[macro_export]
macro_rules! impl_frame_meta_for {
    ($t:ty) => {
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {}

        $crate::check_frame_meta_layout!($t);
    };
}

pub use impl_frame_meta_for;

#[derive(Debug)]
pub enum GetFrameError {
    InUse,
    Unused,
    Busy,
    Unique,
    OutOfBound,
    NotAligned,
}

pub(super) fn get_slot(paddr: Paddr) -> Result<&'static MetaSlot, GetFrameError> {
    if !paddr.is_multiple_of(PAGE_SIZE) {
        return Err(GetFrameError::NotAligned);
    }
    if paddr < super::min_paddr() || paddr >= super::max_paddr() {
        return Err(GetFrameError::OutOfBound);
    }

    let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
    let ptr = vaddr as *mut MetaSlot;

    Ok(unsafe { &*ptr })
}

impl MetaSlot {
    pub(super) fn get_from_unused<M: AnyFrameMeta>(
        paddr: Paddr,
        metadata: M,
        as_unique_ptr: bool,
    ) -> Result<*const Self, GetFrameError> {
        let slot = get_slot(paddr)?;

        slot.ref_count
            .compare_exchange(REF_COUNT_UNUSED, 0, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|val| match val {
                REF_COUNT_UNIQUE => GetFrameError::Unique,
                0 => GetFrameError::Busy,
                _ => GetFrameError::InUse,
            })?;

        unsafe { slot.write_meta(metadata) };

        if as_unique_ptr {
            slot.ref_count.store(REF_COUNT_UNIQUE, Ordering::Relaxed);
        } else {
            slot.ref_count.store(1, Ordering::Release);
        }

        Ok(slot as *const MetaSlot)
    }

    pub(super) fn get_from_in_use(paddr: Paddr) -> Result<*const Self, GetFrameError> {
        let slot = get_slot(paddr)?;

        loop {
            match slot.ref_count.load(Ordering::Relaxed) {
                REF_COUNT_UNUSED => return Err(GetFrameError::Unused),
                REF_COUNT_UNIQUE => return Err(GetFrameError::Unique),
                0 => return Err(GetFrameError::Busy),
                last_ref_cnt => {
                    if last_ref_cnt >= REF_COUNT_MAX {
                        abort();
                    }
                    if slot
                        .ref_count
                        .compare_exchange_weak(
                            last_ref_cnt,
                            last_ref_cnt + 1,
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                    {
                        return Ok(slot as *const MetaSlot);
                    }
                }
            }
            core::hint::spin_loop();
        }
    }

    pub(super) unsafe fn inc_ref_count(&self) {
        let last_ref_cnt = self.ref_count.fetch_add(1, Ordering::Relaxed);
        debug_assert!(last_ref_cnt != 0 && last_ref_cnt != REF_COUNT_UNUSED);

        if last_ref_cnt >= REF_COUNT_MAX {
            abort();
        }
    }

    pub(super) fn frame_paddr(&self) -> Paddr {
        mapping::meta_to_frame::<PagingConsts>(self as *const MetaSlot as Vaddr)
    }

    pub(super) unsafe fn dyn_meta_ptr(&self) -> *mut dyn AnyFrameMeta {
        let vtable_ptr = unsafe { *self.vtable_ptr.get() };

        let vtable_ptr = *unsafe { vtable_ptr.assume_init_ref() };

        let meta_ptr: *mut dyn AnyFrameMeta =
            core::ptr::from_raw_parts_mut(self as *const MetaSlot as *mut MetaSlot, vtable_ptr);

        meta_ptr
    }

    pub(super) fn as_meta_ptr<M: AnyFrameMeta>(&self) -> *mut M {
        self.storage.get() as *mut M
    }

    pub(super) unsafe fn write_meta<M: AnyFrameMeta>(&self, metadata: M) {
        const { assert!(size_of::<M>() <= FRAME_METADATA_MAX_SIZE) };
        const { assert!(align_of::<M>() <= FRAME_METADATA_MAX_ALIGN) };

        let vtable_ptr = unsafe { &mut *self.vtable_ptr.get() };
        vtable_ptr.write(core::ptr::metadata(&metadata as &dyn AnyFrameMeta));

        let ptr = self.storage.get();
        unsafe { ptr.cast::<M>().write(metadata) };
    }

    pub(super) unsafe fn drop_last_in_place(&self) {
        debug_assert_eq!(self.ref_count.load(Ordering::Relaxed), 0);

        unsafe { self.drop_meta_in_place() };

        self.ref_count.store(REF_COUNT_UNUSED, Ordering::Release);
    }

    pub(super) unsafe fn drop_meta_in_place(&self) {
        let paddr = self.frame_paddr();

        let vtable_ptr = unsafe { &mut *self.vtable_ptr.get() };
        let vtable_ptr = unsafe { vtable_ptr.assume_init_read() };

        let meta_ptr: *mut dyn AnyFrameMeta =
            core::ptr::from_raw_parts_mut(self.storage.get(), vtable_ptr);

        let mut reader =
            unsafe { VmReader::from_kernel_space(paddr_to_vaddr(paddr) as *const u8, PAGE_SIZE) };

        unsafe {
            (*meta_ptr).on_drop(&mut reader);
            core::ptr::drop_in_place(meta_ptr);
        }
    }
}

#[derive(Debug, Default)]
pub struct MetaPageMeta {}

impl_frame_meta_for!(MetaPageMeta);

pub(crate) unsafe fn init() -> Segment<MetaPageMeta> {
    let paddr_range = {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        tracked_paddr_range(regions)
    };
    let min_paddr = paddr_range.start;
    let max_paddr = paddr_range.end;

    info!(
        "Initializing frame metadata for physical memory {:x?}",
        paddr_range
    );

    #[cfg(target_arch = "x86_64")]
    add_temp_linear_mapping(max_paddr);

    let tot_nr_frames = paddr_range.len() / page_size::<PagingConsts>(1);
    let (nr_meta_pages, meta_pages) = alloc_meta_frames(tot_nr_frames);

    boot_pt::with_borrow(|boot_pt| {
        for i in 0..nr_meta_pages {
            let frame_paddr = meta_pages + i * PAGE_SIZE;
            let vaddr = mapping::frame_to_meta::<PagingConsts>(min_paddr) + i * PAGE_SIZE;
            let prop = PageProperty {
                flags: PageFlags::RW,
                cache: CachePolicy::Writeback,
                priv_flags: PrivilegedPageFlags::GLOBAL,
            };
            unsafe { boot_pt.map_base_page(vaddr, frame_paddr, prop) };
        }
    })
    .unwrap();

    super::MIN_PADDR.store(min_paddr, Ordering::Relaxed);
    super::MAX_PADDR.store(max_paddr, Ordering::Relaxed);

    let meta_page_range = meta_pages..meta_pages + nr_meta_pages * PAGE_SIZE;

    let early_allocated_ranges = allocator::EARLY_ALLOCATOR
        .lock()
        .as_ref()
        .unwrap()
        .allocated_regions();
    for range in early_allocated_ranges
        .iter()
        .flat_map(|range| range_difference(range, &meta_page_range))
    {
        let early_seg = Segment::from_unused(range, |_| EarlyAllocatedFrameMeta).unwrap();
        let _ = ManuallyDrop::new(early_seg);
    }

    mark_unusable_ranges();

    Segment::from_unused(meta_page_range, |_| MetaPageMeta {}).unwrap()
}

pub(in crate::mm) fn is_initialized() -> bool {
    super::MAX_PADDR.load(Ordering::Relaxed) != 0
}

fn tracked_paddr_range(regions: &[MemoryRegion]) -> Range<Paddr> {
    let physical_regions = regions.iter().filter(|region| region.typ().is_physical());
    let min_paddr = physical_regions
        .clone()
        .map(|region| region.base())
        .min()
        .unwrap();
    let max_paddr = physical_regions.map(|region| region.end()).max().unwrap();

    const META_PAGE_COVERAGE: usize = PAGE_SIZE * (PAGE_SIZE / META_SLOT_SIZE);
    min_paddr.align_down(META_PAGE_COVERAGE)..max_paddr
}

fn alloc_meta_frames(tot_nr_frames: usize) -> (usize, Paddr) {
    let nr_meta_pages = tot_nr_frames
        .checked_mul(size_of::<MetaSlot>())
        .unwrap()
        .div_ceil(PAGE_SIZE);
    let paddr = allocator::early_alloc(
        Layout::from_size_align(nr_meta_pages * PAGE_SIZE, PAGE_SIZE).unwrap(),
    )
    .unwrap();

    let slots = paddr_to_vaddr(paddr) as *mut MetaSlot;

    for i in 0..tot_nr_frames {
        let slot = unsafe { slots.add(i) };
        unsafe {
            slot.write(MetaSlot {
                storage: UnsafeCell::new([0; FRAME_METADATA_MAX_SIZE]),
                ref_count: AtomicU64::new(REF_COUNT_UNUSED),
                vtable_ptr: UnsafeCell::new(MaybeUninit::uninit()),
                in_list: AtomicU64::new(0),
            })
        };
    }

    (nr_meta_pages, paddr)
}

#[derive(Debug)]
pub struct UnusableMemoryMeta;
impl_frame_meta_for!(UnusableMemoryMeta);

#[derive(Debug)]
pub struct ReservedMemoryMeta;
impl_frame_meta_for!(ReservedMemoryMeta);

#[derive(Debug, Default)]
pub struct KernelMeta;
impl_frame_meta_for!(KernelMeta);

macro_rules! mark_ranges {
    ($region: expr, $typ: expr) => {{
        debug_assert!($region.base().is_multiple_of(PAGE_SIZE));
        debug_assert!($region.len().is_multiple_of(PAGE_SIZE));

        let start = $region.base().max(super::min_paddr());
        let seg = Segment::from_unused(start..$region.end(), |_| $typ).unwrap();
        let _ = ManuallyDrop::new(seg);
    }};
}

fn mark_unusable_ranges() {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;

    for region in regions
        .iter()
        .rev()
        .skip_while(|r| !r.typ().is_physical())
        .take_while(|r| r.end() > super::min_paddr())
    {
        match region.typ() {
            MemoryRegionType::BadMemory => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Unknown => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::NonVolatileSleep => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Reserved => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::Kernel => mark_ranges!(region, KernelMeta),
            MemoryRegionType::Module => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Framebuffer => mark_ranges!(region, ReservedMemoryMeta),
            MemoryRegionType::Reclaimable => mark_ranges!(region, UnusableMemoryMeta),
            MemoryRegionType::Usable => {}
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn add_temp_linear_mapping(max_paddr: Paddr) {
    use crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;

    const PADDR4G: Paddr = 0x1_0000_0000;

    if max_paddr <= PADDR4G {
        return;
    }

    let end_paddr = max_paddr.align_up(PAGE_SIZE);
    let prange = PADDR4G..end_paddr;
    let prop = PageProperty {
        flags: PageFlags::RW,
        cache: CachePolicy::Writeback,
        priv_flags: PrivilegedPageFlags::GLOBAL,
    };

    unsafe {
        boot_pt::with_borrow(|boot_pt| {
            for paddr in prange.step_by(PAGE_SIZE) {
                let vaddr = LINEAR_MAPPING_BASE_VADDR + paddr;
                boot_pt.map_base_page(vaddr, paddr, prop);
            }
        })
        .unwrap();
    }
}
