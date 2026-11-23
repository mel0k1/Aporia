use core::{fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use super::util::{
    alloc_kva, cvm_need_private_protection, prepare_dma, split_daddr, unprepare_dma,
};
use crate::{
    arch::{irq, mm::can_sync_dma},
    error::Error,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PAGE_SIZE, Paddr, Split, USegment, VmReader, VmWriter,
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
        kspace::kvirt_area::KVirtArea,
        paddr_to_vaddr,
    },
};

pub trait DmaDirection: 'static + Debug + private::Sealed {
    const CAN_READ_FROM_DEVICE: bool;
    const CAN_WRITE_TO_DEVICE: bool;
}

mod private {
    pub trait Sealed {}
}

#[derive(Debug)]
pub enum ToDevice {}

impl private::Sealed for ToDevice {}
impl DmaDirection for ToDevice {
    const CAN_READ_FROM_DEVICE: bool = false;
    const CAN_WRITE_TO_DEVICE: bool = true;
}

#[derive(Debug)]
pub enum FromDevice {}

impl private::Sealed for FromDevice {}
impl DmaDirection for FromDevice {
    const CAN_READ_FROM_DEVICE: bool = true;
    const CAN_WRITE_TO_DEVICE: bool = false;
}

#[derive(Debug)]
pub enum FromAndToDevice {}

impl private::Sealed for FromAndToDevice {}
impl DmaDirection for FromAndToDevice {
    const CAN_READ_FROM_DEVICE: bool = true;
    const CAN_WRITE_TO_DEVICE: bool = true;
}

#[derive(Debug)]
pub struct DmaStream<D: DmaDirection = FromAndToDevice> {
    inner: Inner,
    map_daddr: Option<Daddr>,
    is_cache_coherent: bool,
    _phantom: PhantomData<D>,
}

#[derive(Debug)]
enum Inner {
    Segment(USegment),
    Kva(KVirtArea, Paddr),
    Both(KVirtArea, Paddr, USegment),
}

impl<D: DmaDirection> DmaStream<D> {
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        const { assert!(D::CAN_WRITE_TO_DEVICE) };

        Self::alloc_uninit(nframes, is_cache_coherent).and_then(|dma| {
            dma.writer()?.fill_zeros(dma.size());
            Ok(dma)
        })
    }

    pub fn alloc_uninit(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        debug_assert!(irq::is_local_enabled());

        let cvm = cvm_need_private_protection();

        let (inner, paddr_range) = if (can_sync_dma() || is_cache_coherent) && !cvm {
            let segment: USegment = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(nframes)?
                .into();
            let paddr_range = segment.paddr_range();

            (Inner::Segment(segment), paddr_range)
        } else {
            let (kva, paddr) = alloc_kva(nframes, can_sync_dma() || is_cache_coherent)?;

            (Inner::Kva(kva, paddr), paddr..paddr + nframes * PAGE_SIZE)
        };

        let map_daddr = unsafe { prepare_dma(&paddr_range) };

        Ok(Self {
            inner,
            map_daddr,
            is_cache_coherent,
            _phantom: PhantomData,
        })
    }

    pub fn map(segment: USegment, is_cache_coherent: bool) -> Result<Self, Error> {
        debug_assert!(irq::is_local_enabled());

        let cvm = cvm_need_private_protection();
        let size = segment.size();

        let (inner, paddr) = if (can_sync_dma() || is_cache_coherent) && !cvm {
            let paddr = segment.paddr();

            (Inner::Segment(segment), paddr)
        } else {
            let (kva, paddr) = alloc_kva(size / PAGE_SIZE, can_sync_dma() || is_cache_coherent)?;

            (Inner::Both(kva, paddr, segment), paddr)
        };

        let paddr_range = paddr..paddr + size;

        let map_daddr = unsafe { prepare_dma(&paddr_range) };

        Ok(Self {
            inner,
            map_daddr,
            is_cache_coherent,
            _phantom: PhantomData,
        })
    }

    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        const { assert!(D::CAN_READ_FROM_DEVICE) };

        self.sync_impl(byte_range, true)
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        const { assert!(D::CAN_WRITE_TO_DEVICE) };

        self.sync_impl(byte_range, false)
    }

    fn sync_impl(&self, byte_range: Range<usize>, is_from_device: bool) -> Result<(), Error> {
        let size = self.size();
        if byte_range.start > byte_range.end || byte_range.end > size {
            return Err(Error::InvalidArgs);
        }

        if !is_from_device && let Inner::Both(kva, _, seg) = &self.inner {
            self.sync_via_copying(byte_range.clone(), false, seg, kva);
        }

        unsafe { self.sync_cache(byte_range.clone(), is_from_device) };

        if is_from_device && let Inner::Both(kva, _, seg) = &self.inner {
            self.sync_via_copying(byte_range, true, seg, kva);
        }

        Ok(())
    }

    unsafe fn sync_cache(&self, byte_range: Range<usize>, is_from_device: bool) {
        if self.is_cache_coherent {
            return;
        }

        let va_range = match &self.inner {
            Inner::Segment(segment) => {
                let pa_range = segment.paddr_range();
                paddr_to_vaddr(pa_range.start)..paddr_to_vaddr(pa_range.end)
            }
            Inner::Kva(kva, _) | Inner::Both(kva, _, _) => {
                if !can_sync_dma() {
                    return;
                }
                kva.range()
            }
        };
        let range = va_range.start + byte_range.start..va_range.start + byte_range.end;

        unsafe { crate::arch::mm::sync_dma_range(range, is_from_device) };
    }

    fn sync_via_copying(
        &self,
        byte_range: Range<usize>,
        is_from_device: bool,
        seg: &USegment,
        kva: &KVirtArea,
    ) {
        let skip = byte_range.start;
        let limit = byte_range.len();

        let (mut reader, mut writer) = if is_from_device {
            let kva_reader =
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) };

            (kva_reader, seg.writer())
        } else {
            let kva_writer =
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) };

            (seg.reader(), kva_writer)
        };

        writer
            .skip(skip)
            .limit(limit)
            .write(reader.skip(skip).limit(limit));
    }
}

impl<D: DmaDirection> Split for DmaStream<D> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset.is_multiple_of(PAGE_SIZE));
        assert!(0 < offset && offset < self.size());

        let (inner, map_daddr, is_cache_coherent) = {
            let this = ManuallyDrop::new(self);
            (
                unsafe { core::ptr::read(&this.inner as *const Inner) },
                this.map_daddr,
                this.is_cache_coherent,
            )
        };

        let (inner1, inner2) = match inner {
            Inner::Segment(segment) => {
                let (s1, s2) = segment.split(offset);
                (Inner::Segment(s1), Inner::Segment(s2))
            }
            Inner::Kva(kva, paddr) => {
                let (kva1, kva2) = kva.split(offset);
                let (paddr1, paddr2) = (paddr, paddr + offset);
                (Inner::Kva(kva1, paddr1), Inner::Kva(kva2, paddr2))
            }
            Inner::Both(kva, paddr, segment) => {
                let (kva1, kva2) = kva.split(offset);
                let (paddr1, paddr2) = (paddr, paddr + offset);
                let (s1, s2) = segment.split(offset);
                (Inner::Both(kva1, paddr1, s1), Inner::Both(kva2, paddr2, s2))
            }
        };

        let (daddr1, daddr2) = split_daddr(map_daddr, offset);

        (
            Self {
                inner: inner1,
                map_daddr: daddr1,
                is_cache_coherent,
                _phantom: PhantomData,
            },
            Self {
                inner: inner2,
                map_daddr: daddr2,
                is_cache_coherent,
                _phantom: PhantomData,
            },
        )
    }
}

impl<D: DmaDirection> Drop for DmaStream<D> {
    fn drop(&mut self) {
        unsafe { unprepare_dma(&self.paddr_range(), self.map_daddr) };
    }
}

impl<D: DmaDirection> HasPaddr for DmaStream<D> {
    fn paddr(&self) -> Paddr {
        match &self.inner {
            Inner::Segment(segment) => segment.paddr(),
            Inner::Kva(_, paddr) | Inner::Both(_, paddr, _) => *paddr,
        }
    }
}

impl<D: DmaDirection> HasDaddr for DmaStream<D> {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl<D: DmaDirection> HasSize for DmaStream<D> {
    fn size(&self) -> usize {
        match &self.inner {
            Inner::Segment(segment) => segment.size(),
            Inner::Kva(kva, _) => kva.size(),
            Inner::Both(kva, _, segment) => {
                debug_assert_eq!(kva.size(), segment.size());
                kva.size()
            }
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaStream<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if !D::CAN_READ_FROM_DEVICE {
            return Err(Error::AccessDenied);
        }
        match &self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.reader()),
            Inner::Kva(kva, _) => unsafe {
                Ok(VmReader::from_kernel_space(
                    kva.start() as *const u8,
                    kva.size(),
                ))
            },
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if !D::CAN_WRITE_TO_DEVICE {
            return Err(Error::AccessDenied);
        }
        match &self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.writer()),
            Inner::Kva(kva, _) => unsafe {
                Ok(VmWriter::from_kernel_space(
                    kva.start() as *mut u8,
                    kva.size(),
                ))
            },
        }
    }
}
