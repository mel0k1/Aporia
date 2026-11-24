//! The unique frame pointer that is not shared with others.

use core::{marker::PhantomData, mem::ManuallyDrop, sync::atomic::Ordering};

use super::{
    AnyFrameMeta, Frame, MetaSlot,
    meta::{GetFrameError, REF_COUNT_UNIQUE},
};
use crate::mm::{HasPaddr, HasSize, PAGE_SIZE, Paddr, PagingConsts, PagingLevel, frame::mapping};

/// An owning frame pointer.
///
/// Unlike [`Frame`], the frame pointed to by this pointer is not shared with
/// others. So a mutable reference to the metadata is available for the frame.
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
    /// Gets a [`UniqueFrame`] with a specific usage from a raw, unused page.
    ///
    /// The caller should provide the initial metadata of the page.
    pub fn from_unused(paddr: Paddr, metadata: M) -> Result<Self, GetFrameError> {
        Ok(Self {
            ptr: MetaSlot::get_from_unused(paddr, metadata, true)?,
            _marker: PhantomData,
        })
    }

    /// Gets the metadata of this page.
    pub fn meta(&self) -> &M {
        unsafe { &*self.slot().as_meta_ptr::<M>() }
    }

    /// Gets the mutable metadata of this page.
    pub fn meta_mut(&mut self) -> &mut M {
        unsafe { &mut *self.slot().as_meta_ptr::<M>() }
    }
}

impl<M: AnyFrameMeta + ?Sized> UniqueFrame<M> {
    /// Gets the paging level of this page.
    ///
    /// This is the level of the page table entry that maps the frame,
    /// which determines the size of the frame.
    ///
    /// Currently, the level is always 1, which means the frame is a regular
    /// page frame.
    pub const fn level(&self) -> PagingLevel {
        1
    }

    /// Gets the dynamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta(&self) -> &dyn AnyFrameMeta {
        unsafe { &*self.slot().dyn_meta_ptr() }
    }

    /// Gets the dynamically-typed metadata of this frame.
    ///
    /// If the type is known at compile time, use [`Frame::meta`] instead.
    pub fn dyn_meta_mut(&mut self) -> &mut dyn AnyFrameMeta {
        unsafe { &mut *self.slot().dyn_meta_ptr() }
    }

    /// Repurposes the frame with a new metadata.
    pub fn repurpose<M1: AnyFrameMeta>(self, metadata: M1) -> UniqueFrame<M1> {
        let this = ManuallyDrop::new(self);

        unsafe { this.slot().drop_meta_in_place() };

        unsafe { this.slot().write_meta(metadata) };

        unsafe { core::mem::transmute(ManuallyDrop::into_inner(this)) }
    }

    /// Resets the frame to unused without up-calling the allocator.
    ///
    /// This is solely useful for the allocator implementation/testing and
    /// is highly experimental. Usage of this function is discouraged.
    ///
    /// Usage of this function other than the allocator would actually leak
    /// the frame since the allocator would not be aware of the frame.
    pub fn reset_as_unused(self) {
        let this = ManuallyDrop::new(self);

        this.slot().ref_count.store(0, Ordering::Release);

        unsafe { this.slot().drop_last_in_place() };
    }

    /// Converts this frame into a raw physical address.
    pub(crate) fn into_raw(self) -> Paddr {
        let this = ManuallyDrop::new(self);
        this.paddr()
    }

    /// Restores a raw physical address back into a unique frame.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the physical address is valid and points to
    /// a forgotten frame that was previously casted by [`Self::into_raw`].
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

    /// Tries to get a unique frame from a shared frame.
    ///
    /// If the reference count is not 1, the frame is returned back.
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
