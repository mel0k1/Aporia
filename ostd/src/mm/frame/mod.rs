pub mod allocator;
pub mod linked_list;
pub mod meta;
pub mod segment;
pub mod unique;
pub mod untyped;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub(crate) mod unaccepted;

mod frame_ref;
pub use frame_ref::FrameRef;

#[cfg(ktest)]
mod test;

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    sync::atomic::{AtomicUsize, Ordering},
};

pub use allocator::GlobalFrameAllocator;
use meta::{AnyFrameMeta, GetFrameError, MetaSlot, REF_COUNT_UNUSED, mapping};
pub use segment::Segment;
use untyped::{AnyUFrameMeta, UFrame};

use crate::{
    mm::{HasPaddr, HasSize, PAGE_SIZE, Paddr, PagingConsts, PagingLevel, Vaddr},
    sync::RcuDrop,
};

static MIN_PADDR: AtomicUsize = AtomicUsize::new(0);
static MAX_PADDR: AtomicUsize = AtomicUsize::new(0);

pub(in crate::mm) fn min_paddr() -> Paddr {
    MIN_PADDR.load(Ordering::Relaxed)
}

pub(in crate::mm) fn max_paddr() -> Paddr {
    let max_paddr = MAX_PADDR.load(Ordering::Relaxed) as Paddr;
    debug_assert_ne!(max_paddr, 0);
    max_paddr
}

#[repr(transparent)]
pub struct Frame<M: AnyFrameMeta + ?Sized> {
    ptr: *const MetaSlot,
    _marker: PhantomData<M>,
}

unsafe impl<M: AnyFrameMeta + ?Sized> Send for Frame<M> {}

unsafe impl<M: AnyFrameMeta + ?Sized> Sync for Frame<M> {}

impl<M: AnyFrameMeta + ?Sized> core::fmt::Debug for Frame<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Frame({:#x})", self.paddr())
    }
}

impl<M: AnyFrameMeta + ?Sized> PartialEq for Frame<M> {
    fn eq(&self, other: &Self) -> bool {
        self.paddr() == other.paddr()
    }
}
impl<M: AnyFrameMeta + ?Sized> Eq for Frame<M> {}

impl<M: AnyFrameMeta> Frame<M> {
    pub fn from_unused(paddr: Paddr, metadata: M) -> Result<Self, GetFrameError> {
        Ok(Self {
            ptr: MetaSlot::get_from_unused(paddr, metadata, false)?,
            _marker: PhantomData,
        })
    }

    pub fn meta(&self) -> &M {
        unsafe { &*self.slot().as_meta_ptr::<M>() }
    }
}

impl Frame<dyn AnyFrameMeta> {
    pub fn from_in_use(paddr: Paddr) -> Result<Self, GetFrameError> {
        Ok(Self {
            ptr: MetaSlot::get_from_in_use(paddr)?,
            _marker: PhantomData,
        })
    }
}

impl<M: AnyFrameMeta + ?Sized> Frame<M> {
    pub const fn map_level(&self) -> PagingLevel {
        1
    }

    pub fn dyn_meta(&self) -> &dyn AnyFrameMeta {
        unsafe { &*self.slot().dyn_meta_ptr() }
    }

    pub fn reference_count(&self) -> u64 {
        let refcnt = self.slot().ref_count.load(Ordering::Relaxed);
        debug_assert!(refcnt < meta::REF_COUNT_MAX);
        refcnt
    }

    pub fn borrow(&self) -> FrameRef<'_, M> {
        unsafe { FrameRef::borrow_paddr(self.paddr()) }
    }

    pub(in crate::mm) fn into_raw(self) -> Paddr {
        let this = ManuallyDrop::new(self);
        this.paddr()
    }

    pub(crate) unsafe fn from_raw(paddr: Paddr) -> Self {
        debug_assert!(paddr < max_paddr());

        let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn slot(&self) -> &MetaSlot {
        unsafe { &*self.ptr }
    }
}

impl<M: AnyFrameMeta + ?Sized> HasPaddr for Frame<M> {
    fn paddr(&self) -> Paddr {
        self.slot().frame_paddr()
    }
}

impl<M: AnyFrameMeta + ?Sized> HasSize for Frame<M> {
    fn size(&self) -> usize {
        PAGE_SIZE
    }
}

impl<M: AnyFrameMeta + ?Sized> Clone for Frame<M> {
    fn clone(&self) -> Self {
        unsafe { self.slot().inc_ref_count() };

        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Drop for Frame<M> {
    fn drop(&mut self) {
        let last_ref_cnt = self.slot().ref_count.fetch_sub(1, Ordering::Release);
        debug_assert!(last_ref_cnt != 0 && last_ref_cnt != REF_COUNT_UNUSED);

        if last_ref_cnt == 1 {
            core::sync::atomic::fence(Ordering::Acquire);

            unsafe { self.slot().drop_last_in_place() };

            allocator::get_global_frame_allocator().dealloc(self.paddr(), PAGE_SIZE);
        }
    }
}

impl<M: AnyFrameMeta> TryFrom<Frame<dyn AnyFrameMeta>> for Frame<M> {
    type Error = Frame<dyn AnyFrameMeta>;

    fn try_from(dyn_frame: Frame<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        if (dyn_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            Ok(unsafe { core::mem::transmute::<Frame<dyn AnyFrameMeta>, Frame<M>>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

impl Frame<dyn AnyFrameMeta> {
    pub fn from_unsized<M: AnyFrameMeta + ?Sized>(frame: Frame<M>) -> Frame<dyn AnyFrameMeta> {
        unsafe { core::mem::transmute(frame) }
    }

    pub fn rcu_from_unsized<M: AnyFrameMeta + ?Sized>(frame: RcuDrop<Frame<M>>) -> RcuDrop<Self> {
        let (frame, panic_guard) = unsafe { RcuDrop::into_inner(frame) };
        let dyn_frame = Self::from_unsized(frame);
        panic_guard.forget();
        RcuDrop::new(dyn_frame)
    }
}

impl<M: AnyFrameMeta> From<Frame<M>> for Frame<dyn AnyFrameMeta> {
    fn from(frame: Frame<M>) -> Self {
        Self::from_unsized(frame)
    }
}

impl<M: AnyUFrameMeta> From<Frame<M>> for UFrame {
    fn from(frame: Frame<M>) -> Self {
        unsafe { core::mem::transmute(frame) }
    }
}

impl From<UFrame> for Frame<dyn AnyFrameMeta> {
    fn from(frame: UFrame) -> Self {
        unsafe { core::mem::transmute(frame) }
    }
}

impl TryFrom<Frame<dyn AnyFrameMeta>> for UFrame {
    type Error = Frame<dyn AnyFrameMeta>;

    fn try_from(dyn_frame: Frame<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        if dyn_frame.dyn_meta().is_untyped() {
            Ok(unsafe { core::mem::transmute::<Frame<dyn AnyFrameMeta>, UFrame>(dyn_frame) })
        } else {
            Err(dyn_frame)
        }
    }
}

pub(in crate::mm) unsafe fn inc_frame_ref_count(paddr: Paddr) {
    debug_assert!(paddr.is_multiple_of(PAGE_SIZE));
    debug_assert!(paddr < max_paddr());

    let vaddr: Vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
    let slot = unsafe { &*(vaddr as *const MetaSlot) };

    unsafe { slot.inc_ref_count() };
}
