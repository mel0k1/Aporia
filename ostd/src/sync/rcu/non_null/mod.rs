mod either;

use core::{marker::PhantomData, mem::ManuallyDrop, ops::Deref, ptr::NonNull};

use crate::prelude::*;

pub unsafe trait NonNullPtr: 'static {
    type Target;

    type Ref<'a>
    where
        Self: 'a;

    const ALIGN_BITS: u32;

    fn into_raw(self) -> NonNull<Self::Target>;

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self;

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a>;

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target>;
}

#[derive(Debug)]
pub struct BoxRef<'a, T> {
    inner: *mut T,
    _marker: PhantomData<&'a T>,
}

impl<T> Deref for BoxRef<'_, T> {
    type Target = Box<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(&self.inner) }
    }
}

impl<'a, T> BoxRef<'a, T> {
    pub fn deref_target(&self) -> &'a T {
        unsafe { &*(self.inner) }
    }
}

unsafe impl<T: 'static> NonNullPtr for Box<T> {
    type Target = T;

    type Ref<'a>
        = BoxRef<'a, T>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = align_of::<T>().trailing_zeros();

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = Box::into_raw(self);

        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self {
        let ptr = ptr.as_ptr();

        unsafe { Box::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        BoxRef {
            inner: raw.as_ptr(),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        unsafe { NonNull::new_unchecked(ptr_ref.inner) }
    }
}

#[derive(Debug)]
pub struct ArcRef<'a, T> {
    inner: ManuallyDrop<Arc<T>>,
    _marker: PhantomData<&'a Arc<T>>,
}

impl<T> Deref for ArcRef<'_, T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, T> ArcRef<'a, T> {
    pub fn deref_target(&self) -> &'a T {
        unsafe { &*(self.deref().deref() as *const T) }
    }
}

unsafe impl<T: 'static> NonNullPtr for Arc<T> {
    type Target = T;

    type Ref<'a>
        = ArcRef<'a, T>
    where
        Self: 'a;

    const ALIGN_BITS: u32 = align_of::<T>().trailing_zeros();

    fn into_raw(self) -> NonNull<Self::Target> {
        let ptr = Arc::into_raw(self).cast_mut();

        unsafe { NonNull::new_unchecked(ptr) }
    }

    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self {
        let ptr = ptr.as_ptr().cast_const();

        unsafe { Arc::from_raw(ptr) }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<Self::Target>) -> Self::Ref<'a> {
        unsafe {
            ArcRef {
                inner: ManuallyDrop::new(Arc::from_raw(raw.as_ptr())),
                _marker: PhantomData,
            }
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<Self::Target> {
        NonNullPtr::into_raw(ManuallyDrop::into_inner(ptr_ref.inner))
    }
}
