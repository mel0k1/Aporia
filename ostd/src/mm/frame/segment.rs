use core::{fmt::Debug, mem::ManuallyDrop, ops::Range};

use super::{
    Frame, inc_frame_ref_count,
    meta::{AnyFrameMeta, GetFrameError},
};
use crate::mm::{AnyUFrameMeta, HasPaddr, HasSize, PAGE_SIZE, Paddr, Split};

#[repr(transparent)]
pub struct Segment<M: AnyFrameMeta + ?Sized> {
    range: Range<Paddr>,
    _marker: core::marker::PhantomData<M>,
}

impl<M: AnyFrameMeta + ?Sized> Debug for Segment<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Segment({:#x}..{:#x})", self.range.start, self.range.end)
    }
}

pub type USegment = Segment<dyn AnyUFrameMeta>;

impl<M: AnyFrameMeta + ?Sized> Drop for Segment<M> {
    fn drop(&mut self) {
        unsafe { release_frames::<M>(&self.range) };
    }
}

unsafe fn release_frames<M: AnyFrameMeta + ?Sized>(range: &Range<Paddr>) {
    for paddr in range.clone().step_by(PAGE_SIZE) {
        drop(unsafe { Frame::<M>::from_raw(paddr) });
    }
}

impl<M: AnyFrameMeta + ?Sized> Clone for Segment<M> {
    fn clone(&self) -> Self {
        for paddr in self.range.clone().step_by(PAGE_SIZE) {
            unsafe { inc_frame_ref_count(paddr) };
        }
        Self {
            range: self.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta> Segment<M> {
    pub fn from_unused<F>(range: Range<Paddr>, mut metadata_fn: F) -> Result<Self, GetFrameError>
    where
        F: FnMut(Paddr) -> M,
    {
        if !range.start.is_multiple_of(PAGE_SIZE) || !range.end.is_multiple_of(PAGE_SIZE) {
            return Err(GetFrameError::NotAligned);
        }
        if range.end > super::max_paddr() {
            return Err(GetFrameError::OutOfBound);
        }
        assert!(range.start < range.end);
        let mut paddr_iter = range.step_by(PAGE_SIZE);
        let mut segment = {
            let first_paddr = paddr_iter.next().unwrap();
            Self::from(Frame::<M>::from_unused(
                first_paddr,
                metadata_fn(first_paddr),
            )?)
        };
        for paddr in paddr_iter {
            let frame = Frame::<M>::from_unused(paddr, metadata_fn(paddr))?;
            let _ = ManuallyDrop::new(frame);
            segment.range.end = paddr + PAGE_SIZE;
        }
        Ok(segment)
    }

    pub(crate) unsafe fn from_raw(range: Range<Paddr>) -> Self {
        debug_assert_eq!(range.start % PAGE_SIZE, 0);
        debug_assert_eq!(range.end % PAGE_SIZE, 0);
        debug_assert!(range.start < range.end);
        Self {
            range,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> Split for Segment<M> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(
            offset.is_multiple_of(PAGE_SIZE),
            "segment virtual address not aligned for splitting"
        );
        assert!(
            0 < offset && offset < self.size(),
            "segment virtual address out-of-bound for splitting"
        );

        let old = ManuallyDrop::new(self);
        let at = old.range.start + offset;

        (
            Self {
                range: old.range.start..at,
                _marker: core::marker::PhantomData,
            },
            Self {
                range: at..old.range.end,
                _marker: core::marker::PhantomData,
            },
        )
    }
}

impl<M: AnyFrameMeta + ?Sized> Segment<M> {
    pub fn slice(&self, range: &Range<usize>) -> Self {
        assert!(
            range.start.is_multiple_of(PAGE_SIZE) && range.end.is_multiple_of(PAGE_SIZE),
            "segment virtual address not aligned for slicing"
        );
        assert!(
            range.end <= self.size(),
            "segment virtual address out-of-bound for slicing"
        );
        assert!(
            range.start < range.end,
            "segment virtual address empty for slicing"
        );

        let start = self.range.start + range.start;
        let end = self.range.start + range.end;

        for paddr in (start..end).step_by(PAGE_SIZE) {
            unsafe { inc_frame_ref_count(paddr) };
        }

        Self {
            range: start..end,
            _marker: core::marker::PhantomData,
        }
    }

    pub(crate) fn into_raw(self) -> Range<Paddr> {
        let range = self.range.clone();
        let _ = ManuallyDrop::new(self);
        range
    }
}

impl Segment<dyn AnyFrameMeta> {
    pub fn from_unsized<M: AnyFrameMeta + ?Sized>(
        segment: Segment<M>,
    ) -> Segment<dyn AnyFrameMeta> {
        let seg = ManuallyDrop::new(segment);
        Self {
            range: seg.range.clone(),
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta + ?Sized> HasPaddr for Segment<M> {
    fn paddr(&self) -> Paddr {
        self.range.start
    }
}

impl<M: AnyFrameMeta + ?Sized> HasSize for Segment<M> {
    fn size(&self) -> usize {
        self.range.end - self.range.start
    }
}

impl<M: AnyFrameMeta + ?Sized> From<Frame<M>> for Segment<M> {
    fn from(frame: Frame<M>) -> Self {
        let pa = frame.paddr();
        let _ = ManuallyDrop::new(frame);
        Self {
            range: pa..pa + PAGE_SIZE,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<M: AnyFrameMeta> From<Segment<M>> for Segment<dyn AnyFrameMeta> {
    fn from(seg: Segment<M>) -> Self {
        Self::from_unsized(seg)
    }
}

impl<M: AnyFrameMeta> TryFrom<Segment<dyn AnyFrameMeta>> for Segment<M> {
    type Error = Segment<dyn AnyFrameMeta>;

    fn try_from(seg: Segment<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        let first_frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !(first_frame.dyn_meta() as &dyn core::any::Any).is::<M>() {
            return Err(seg);
        }
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!((frame.dyn_meta() as &dyn core::any::Any).is::<M>());
            }
        }
        Ok(unsafe { core::mem::transmute::<Segment<dyn AnyFrameMeta>, Segment<M>>(seg) })
    }
}

impl<M: AnyUFrameMeta> From<Segment<M>> for USegment {
    fn from(seg: Segment<M>) -> Self {
        unsafe { core::mem::transmute(seg) }
    }
}

impl TryFrom<Segment<dyn AnyFrameMeta>> for USegment {
    type Error = Segment<dyn AnyFrameMeta>;

    fn try_from(seg: Segment<dyn AnyFrameMeta>) -> Result<Self, Self::Error> {
        let first_frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(seg.range.start) };
        let first_frame = ManuallyDrop::new(first_frame);
        if !first_frame.dyn_meta().is_untyped() {
            return Err(seg);
        }
        #[cfg(debug_assertions)]
        {
            for paddr in seg.range.clone().step_by(PAGE_SIZE) {
                let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
                let frame = ManuallyDrop::new(frame);
                debug_assert!(frame.dyn_meta().is_untyped());
            }
        }
        Ok(unsafe { core::mem::transmute::<Segment<dyn AnyFrameMeta>, USegment>(seg) })
    }
}

pub use self::iter::IntoIter;

mod iter {
    use core::{marker::PhantomData, ops::Range};

    use super::{Frame, Segment};
    use crate::mm::{PAGE_SIZE, Paddr, frame::meta::AnyFrameMeta};

    pub struct IntoIter<M: AnyFrameMeta + ?Sized> {
        range: Range<Paddr>,
        _marker: PhantomData<M>,
    }

    impl<M: AnyFrameMeta + ?Sized> Iterator for IntoIter<M> {
        type Item = Frame<M>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.range.start < self.range.end {
                let frame = unsafe { Frame::<M>::from_raw(self.range.start) };
                self.range.start += PAGE_SIZE;
                debug_assert!(self.range.start <= self.range.end);
                Some(frame)
            } else {
                None
            }
        }
    }

    impl<M: AnyFrameMeta + ?Sized> Drop for IntoIter<M> {
        fn drop(&mut self) {
            unsafe { super::release_frames::<M>(&self.range) };
        }
    }

    impl<M: AnyFrameMeta + ?Sized> IntoIterator for Segment<M> {
        type Item = Frame<M>;
        type IntoIter = IntoIter<M>;

        fn into_iter(self) -> Self::IntoIter {
            IntoIter {
                range: self.into_raw(),
                _marker: PhantomData,
            }
        }
    }
}
