use core::{marker::PhantomData, mem::ManuallyDrop, sync::atomic::Ordering};

use super::{
    AnyFrameMeta, Frame, MetaSlot,
    meta::{GetFrameError, REF_COUNT_UNIQUE},
};
use crate::mm::{HasPaddr, HasSize, PAGE_SIZE, Paddr, PagingConsts, PagingLevel, frame::mapping};

#[repr(transparent)]
pub struct UniqueFrame<M: AnyFrameMeta + ?Sized> {
    ptr: *const MetaSlot,
    _marker: PhantomData<M>,
}

unsafe impl<M: AnyFrameMeta + ?Sized> Send for UniqueFrame<M> {}

unsafe impl<M: AnyFrameMeta + ?Sized> Sync for UniqueFrame<M> {}

impl<M: AnyFrameMeta + ?Sized> core::fmt::Debug for UniqueFrame<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UniqueFrame({:#x})", self.paddr())
    }
}

impl<M: AnyFrameMeta> UniqueFrame<M> {
    pub fn from_unused(paddr: Paddr, metadata: M) -> Result<Self, GetFrameError> {
        Ok(Self {
            ptr: MetaSlot::get_from_unused(paddr, metadata, true)?,
            _marker: PhantomData,
        })
    }

    pub fn meta(&self) -> &M {
        unsafe { &*self.slot().as_meta_ptr::<M>() }
    }

    pub fn meta_mut(&mut self) -> &mut M {
        unsafe { &mut *self.slot().as_meta_ptr::<M>() }
    }
}

impl<M: AnyFrameMeta + ?Sized> UniqueFrame<M> {
    pub const fn level(&self) -> PagingLevel {
        1
    }

    pub fn dyn_meta(&self) -> &dyn AnyFrameMeta {
        unsafe { &*self.slot().dyn_meta_ptr() }
    }

    pub fn dyn_meta_mut(&mut self) -> &mut dyn AnyFrameMeta {
        unsafe { &mut *self.slot().dyn_meta_ptr() }
    }

    pub fn repurpose<M1: AnyFrameMeta>(self, metadata: M1) -> UniqueFrame<M1> {
        let this = ManuallyDrop::new(self);

        unsafe { this.slot().drop_meta_in_place() };
        unsafe { this.slot().write_meta(metadata) };
        unsafe { core::mem::transmute(ManuallyDrop::into_inner(this)) }
    }

    pub fn reset_as_unused(self) {
        let this = ManuallyDrop::new(self);

        this.slot().ref_count.store(0, Ordering::Release);
        unsafe { this.slot().drop_last_in_place() };
    }

    pub(crate) fn into_raw(self) -> Paddr {
        let this = ManuallyDrop::new(self);
        this.paddr()
    }

    pub(crate) unsafe fn from_raw(paddr: Paddr) -> Self {
        let vaddr = mapping::frame_to_meta::<PagingConsts>(paddr);
        let ptr = vaddr as *const MetaSlot;

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    pub(super) fn slot(&self) -> &MetaSlot {
        unsafe { &*self.ptr }
    }
}

impl<M: AnyFrameMeta + ?Sized> HasPaddr for UniqueFrame<M> {
    fn paddr(&self) -> Paddr {
        self.slot().frame_paddr()
    }
}

impl<M: AnyFrameMeta + ?Sized> HasSize for UniqueFrame<M> {
    fn size(&self) -> usize {
        PAGE_SIZE
    }
}

impl<M: AnyFrameMeta + ?Sized> Drop for UniqueFrame<M> {
    fn drop(&mut self) {
        self.slot().ref_count.store(0, Ordering::Relaxed);
        unsafe { self.slot().drop_last_in_place() };

        super::allocator::get_global_frame_allocator().dealloc(self.paddr(), PAGE_SIZE);
    }
}

impl<M: AnyFrameMeta + ?Sized> From<UniqueFrame<M>> for Frame<M> {
    fn from(unique: UniqueFrame<M>) -> Self {
        unique.slot().ref_count.store(1, Ordering::Release);
        unsafe { core::mem::transmute(unique) }
    }
}

impl<M: AnyFrameMeta + ?Sized> TryFrom<Frame<M>> for UniqueFrame<M> {
    type Error = Frame<M>;

    fn try_from(frame: Frame<M>) -> Result<Self, Self::Error> {
        match frame.slot().ref_count.compare_exchange(
            1,
            REF_COUNT_UNIQUE,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_) => Ok(unsafe { core::mem::transmute::<Frame<M>, UniqueFrame<M>>(frame) }),
            Err(_) => Err(frame),
        }
    }
}
