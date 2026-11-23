use alloc::sync::Arc;

use aster_util::mem_obj_slice::Slice;
use dma_pool::{DmaArenaPool, DmaBuffer, DmaPool};
use ostd::{
    Result,
    mm::{
        HasSize, Infallible, PAGE_SIZE, USegment, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
};
use smallvec::SmallVec;
use spin::Once;

use crate::dma_buf::DmaBuf;

const MIN_SHIFT: usize = 6;

const MAX_SHIFT: usize = 12;

const N_CLASSES: usize = MAX_SHIFT - MIN_SHIFT + 1;

const MAX_CLASS_SIZE: usize = 1 << MAX_SHIFT;

const POOL_INIT_SIZE: usize = 8;

const POOL_HIGH_WATERMARK: usize = 64;

const DMA_ARENA_SIZE_PAGES: usize = 8 * 1024 * 1024 / PAGE_SIZE;

static VIRTIOFS_DMA_ARENA_RPOOL: Once<Arc<DmaArenaPool<FromDevice>>> = Once::new();
static VIRTIOFS_DMA_ARENA_WPOOL: Once<Arc<DmaArenaPool<ToDevice>>> = Once::new();

pub(super) fn dma_arena_pools_singleton() -> (
    &'static Arc<DmaArenaPool<ToDevice>>,
    &'static Arc<DmaArenaPool<FromDevice>>,
) {
    VIRTIOFS_DMA_ARENA_RPOOL.call_once(|| DmaArenaPool::new(DMA_ARENA_SIZE_PAGES).unwrap());
    VIRTIOFS_DMA_ARENA_WPOOL.call_once(|| DmaArenaPool::new(DMA_ARENA_SIZE_PAGES).unwrap());

    (
        VIRTIOFS_DMA_ARENA_WPOOL.get().unwrap(),
        VIRTIOFS_DMA_ARENA_RPOOL.get().unwrap(),
    )
}

#[derive(Debug)]
pub(super) struct VirtiofsDmaPool<D: DmaDirection> {
    classes: [Arc<DmaPool<D>>; N_CLASSES],
    arena_pool: &'static Arc<DmaArenaPool<D>>,
}

impl<D: DmaDirection> VirtiofsDmaPool<D> {
    pub(super) fn new(arena_pool: &'static Arc<DmaArenaPool<D>>) -> Self {
        let classes = core::array::from_fn(|i| {
            let segment_size = 1 << (MIN_SHIFT + i);
            DmaPool::<D>::new(segment_size, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false)
        });

        Self {
            classes,
            arena_pool,
        }
    }

    fn alloc_buf(&self, len: usize) -> Result<Arc<Slice<DmaBuffer<D>>>> {
        if len == 0 {
            return Err(ostd::Error::InvalidArgs);
        }

        let storage = if len <= MAX_CLASS_SIZE {
            let shift = MIN_SHIFT.max(len.next_power_of_two().trailing_zeros() as usize);
            DmaBuffer::Pooled(self.classes[shift - MIN_SHIFT].alloc_segment()?)
        } else {
            let pages = len.div_ceil(PAGE_SIZE);
            match self.arena_pool.alloc(pages) {
                Some(arena) => DmaBuffer::Arena(arena),
                None => DmaBuffer::Direct(DmaStream::alloc_uninit(pages, false)?),
            }
        };

        Ok(Arc::new(Slice::new(storage, 0..len)))
    }
}

impl VirtiofsDmaPool<FromDevice> {
    pub(super) fn alloc_reply_buf(&self, len: usize) -> Result<FuseReplyBuf> {
        self.alloc_buf(len).map(FuseReplyBuf)
    }
}

impl VirtiofsDmaPool<ToDevice> {
    pub(super) fn alloc_request_buf(&self, len: usize) -> Result<FuseRequestBuf> {
        self.alloc_buf(len).map(FuseRequestBuf)
    }
}

pub(super) enum FuseDataBuf {
    Read(FuseReplyBufs),
    Write(FuseRequestBufs),
}

#[derive(Clone, Debug)]
pub struct FuseRequestBuf(Arc<Slice<DmaBuffer<ToDevice>>>);

pub type FuseRequestBufs = SmallVec<[FuseRequestBuf; 1]>;

impl FuseRequestBuf {
    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    pub(crate) fn as_dma_slice(&self) -> &Slice<DmaBuffer<ToDevice>> {
        self.0.as_ref()
    }

    pub(crate) fn sync_to_device(&self) -> Result<()> {
        self.0.mem_obj().sync_to_device(self.0.offset().clone())
    }
}

impl HasVmReaderWriter for FuseRequestBuf {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        self.0.reader()
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        self.0.writer()
    }
}

#[derive(Clone, Debug)]
pub struct FuseReplyBuf(Arc<Slice<DmaBuffer<FromDevice>>>);

pub type FuseReplyBufs = SmallVec<[FuseReplyBuf; 1]>;

impl FuseReplyBuf {
    pub fn new_map(segment: USegment) -> Result<Self> {
        let len = segment.size();
        let stream = DmaStream::map(segment, false)?;

        Ok(FuseReplyBuf(Arc::new(Slice::new(
            DmaBuffer::Direct(stream),
            0..len,
        ))))
    }

    pub(crate) fn len(&self) -> usize {
        DmaBuf::len(self.0.as_ref())
    }

    pub(crate) fn as_dma_slice(&self) -> &Slice<DmaBuffer<FromDevice>> {
        self.0.as_ref()
    }

    pub(crate) fn sync_from_device(&self) -> Result<()> {
        self.0.mem_obj().sync_from_device(self.0.offset().clone())
    }
}

impl HasVmReaderWriter for FuseReplyBuf {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        self.0.reader()
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        self.0.writer()
    }
}
