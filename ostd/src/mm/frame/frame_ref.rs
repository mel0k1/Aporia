use core::{fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

use super::{
    Frame,
    meta::{AnyFrameMeta, MetaSlot},
};
use crate::{
    mm::{HasPaddr, Paddr},
    sync::non_null::NonNullPtr,
};

pub struct FrameRef<'a, M: AnyFrameMeta + ?Sized> {
    inner: ManuallyDrop<Frame<M>>,
    _marker: PhantomData<&'a Frame<M>>,
}

impl<M: AnyFrameMeta + ?Sized> FrameRef<'_, M> {
    pub(crate) unsafe fn borrow_paddr(raw: Paddr) -> Self {
        Self {
            inner: ManuallyDrop::new(unsafe { Frame::from_raw(raw) }),
            _marker: PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Deref for FrameRef<'_, M> {
    type Target = Frame<M>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<M: AnyFrameMeta + ?Sized> Debug for FrameRef<'_, M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameRef")
            .field("paddr", &self.inner.paddr())
            .finish()
    }
}

unsafe impl<M: AnyFrameMeta + ?Sized> NonNullPtr for Frame<M> {
    type Target = PhantomData<Self>;

    type Ref<'a>
        = FrameRef<'a, M>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = align_of::<MetaSlot>().trailing_zeros();

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = NonNull::new(self.ptr.cast_mut()).unwrap();
        let _ = ManuallyDrop::new(self);
        ptr.cast()
    }

    unsafe fn from_raw(raw: NonNull<Self::Target>) -> Self {
        Self {
            ptr: raw.as_ptr().cast_const().cast(),
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        Self::Ref {
            inner: ManuallyDrop::new(Frame {
                ptr: raw.as_ptr().cast_const().cast(),
                _marker: PhantomData,
            }),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        NonNull::new(ptr_ref.inner.ptr.cast_mut()).unwrap().cast()
    }
}
