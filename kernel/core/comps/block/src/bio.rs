use alloc::boxed::Box;

use align_ext::AlignExt;
use aster_util::mem_obj_slice::Slice;
use dma_pool::{DmaArenaPool, DmaBuffer};
use int_to_c_enum::TryFromInt;
use io_util::{
    IoError,
    batch::{IoBatch, IoCompletion},
};
use ostd::{
    Error,
    mm::{
        HasSize, Infallible, USegment, VmReader, VmWriter,
        dma::{DmaStream, FromAndToDevice},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
    sync::WaitQueue,
};
use spin::Once;

use super::{BlockDevice, id::Sid};
use crate::{BLOCK_SIZE, SECTOR_SIZE, impl_block_device::general_complete_fn, prelude::*};

pub struct Bio {
    metadata: Arc<BioMetadata>,
    complete_fn: Option<BioCompleteFn>,
    segments: Vec<BioSegment>,
}

pub type BioCompleteFn = Box<dyn FnOnce(BioStatus) + Send>;

impl Bio {
    pub fn new(
        type_: BioType,
        start_sid: Sid,
        segments: Vec<BioSegment>,
        complete_fn: Option<BioCompleteFn>,
    ) -> Self {
        let nsectors = segments
            .iter()
            .map(|segment| segment.nsectors().to_raw())
            .sum();

        let metadata = Arc::new(BioMetadata {
            type_,
            sid_range: start_sid..start_sid + nsectors,
            status: AtomicU32::new(BioStatus::Init as u32),
            wait_queue: WaitQueue::new(),
        });
        Self {
            metadata,
            complete_fn,
            segments,
        }
    }

    pub fn type_(&self) -> BioType {
        self.metadata.type_()
    }

    pub fn sid_range(&self) -> &Range<Sid> {
        self.metadata.sid_range()
    }

    pub fn segments(&self) -> &[BioSegment] {
        &self.segments
    }

    pub fn status(&self) -> BioStatus {
        self.metadata.status()
    }

    pub fn submit(
        self,
        block_device: &dyn BlockDevice,
        io_batch: &mut IoBatch,
    ) -> Result<(), BioEnqueueError> {
        let Self {
            metadata,
            complete_fn,
            segments,
        } = self;

        let result = metadata.status.compare_exchange(
            BioStatus::Init as u32,
            BioStatus::Submit as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        let waiter_metadata = metadata.clone();
        let submitted_bio = SubmittedBio {
            metadata,
            sid_offset: 0,
            complete_fn,
            segments,
        };
        if let Err(e) = block_device.enqueue(submitted_bio) {
            let result = waiter_metadata.status.compare_exchange(
                BioStatus::Submit as u32,
                BioStatus::Init as u32,
                Ordering::Release,
                Ordering::Relaxed,
            );
            assert!(result.is_ok());
            return Err(e);
        }

        io_batch.push(waiter_metadata);
        Ok(())
    }

    pub fn submit_and_wait(
        self,
        block_device: &dyn BlockDevice,
    ) -> Result<BioStatus, BioEnqueueError> {
        let mut io_batch = IoBatch::with_capacity(1);
        self.submit(block_device, &mut io_batch)?;
        let _ = io_batch.wait_all();
        let metadata = io_batch[0].downcast_ref::<BioMetadata>().unwrap();
        Ok(metadata.status())
    }
}

impl Debug for Bio {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Bio")
            .field("metadata", &self.metadata)
            .field("segments", &self.segments)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BioEnqueueError {
    IsFull,
    Refused,
    TooBig,
}

impl From<BioEnqueueError> for Error {
    fn from(_error: BioEnqueueError) -> Self {
        Error::NotEnoughResources
    }
}

pub struct SubmittedBio {
    metadata: Arc<BioMetadata>,
    sid_offset: u64,
    complete_fn: Option<BioCompleteFn>,
    segments: Vec<BioSegment>,
}

impl SubmittedBio {
    pub fn type_(&self) -> BioType {
        self.metadata.type_()
    }

    pub fn sid_range(&self) -> &Range<Sid> {
        self.metadata.sid_range()
    }

    pub fn sid_offset(&self) -> u64 {
        self.sid_offset
    }

    pub fn set_sid_offset(&mut self, offset: u64) {
        self.sid_offset = offset;
    }

    pub fn segments(&self) -> &[BioSegment] {
        &self.segments
    }

    pub fn status(&self) -> BioStatus {
        self.metadata.status()
    }

    pub fn complete(self, status: BioStatus) {
        assert!(status != BioStatus::Init && status != BioStatus::Submit);

        let Self {
            metadata,
            complete_fn,
            segments,
            ..
        } = self;

        drop(segments);

        general_complete_fn(metadata.type_(), status, complete_fn);

        let result = metadata.status.compare_exchange(
            BioStatus::Submit as u32,
            status as u32,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(result.is_ok());

        metadata.wait_queue.wake_all();
    }
}

impl Debug for SubmittedBio {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("SubmittedBio")
            .field("metadata", &self.metadata)
            .field("sid_offset", &self.sid_offset)
            .field("segments", &self.segments)
            .finish()
    }
}

struct BioMetadata {
    type_: BioType,
    sid_range: Range<Sid>,
    status: AtomicU32,
    wait_queue: WaitQueue,
}

impl BioMetadata {
    fn type_(&self) -> BioType {
        self.type_
    }

    fn sid_range(&self) -> &Range<Sid> {
        &self.sid_range
    }

    fn status(&self) -> BioStatus {
        BioStatus::try_from(self.status.load(Ordering::Acquire)).unwrap()
    }
}

impl IoCompletion for BioMetadata {
    fn wait(&self) -> Result<(), IoError> {
        let status = self.wait_queue.wait_until(|| {
            let status = self.status();
            (status != BioStatus::Submit).then_some(status)
        });

        match status {
            BioStatus::Complete => Ok(()),
            BioStatus::NotSupported => Err(IoError::Unsupported),
            BioStatus::NoSpace => Err(IoError::OutOfSpace),
            BioStatus::IoError => Err(IoError::Failed),
            BioStatus::Init | BioStatus::Submit | BioStatus::Zeros => unreachable!(),
        }
    }
}

impl Debug for BioMetadata {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BioMetadata")
            .field("type", &self.type_())
            .field("sid_range", &self.sid_range())
            .field("status", &self.status())
            .finish()
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, TryFromInt)]
pub enum BioType {
    Read = 0,
    Write = 1,
    Flush = 2,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum BioStatus {
    Init = 0,
    Submit = 1,
    Complete = 2,
    NotSupported = 3,
    NoSpace = 4,
    IoError = 5,
    Zeros = 6,
}

#[derive(Clone, Debug)]
pub struct BioSegment {
    inner: Arc<BioSegmentInner>,
}

#[derive(Debug)]
struct BioSegmentInner {
    storage: Slice<DmaBuffer<FromAndToDevice>>,
    direction: BioDirection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BioDirection {
    FromDevice,
    ToDevice,
}

impl BioSegment {
    pub fn alloc(nblocks: usize, direction: BioDirection) -> Self {
        Self::alloc_inner(nblocks, 0, nblocks * BLOCK_SIZE, direction)
    }

    pub(super) fn alloc_inner(
        nblocks: usize,
        offset_within_first_block: usize,
        len: usize,
        direction: BioDirection,
    ) -> Self {
        let offset = offset_within_first_block;
        assert!(
            is_sector_aligned(offset)
                && offset < BLOCK_SIZE
                && is_sector_aligned(len)
                && offset + len <= nblocks * BLOCK_SIZE
        );

        let bio_segment_inner = target_pool(direction)
            .and_then(|pool| pool.alloc(nblocks, offset, len))
            .unwrap_or_else(|| {
                let dma_stream = DmaStream::alloc_uninit(nblocks, false).unwrap();
                BioSegmentInner {
                    storage: Slice::new(DmaBuffer::Direct(dma_stream), offset..offset + len),
                    direction,
                }
            });

        Self {
            inner: Arc::new(bio_segment_inner),
        }
    }

    pub fn new_from_segment(segment: USegment, direction: BioDirection) -> Self {
        let len = segment.size();
        let dma_stream = DmaStream::map(segment, false).unwrap();
        Self {
            inner: Arc::new(BioSegmentInner {
                storage: Slice::new(DmaBuffer::Direct(dma_stream), 0..len),
                direction,
            }),
        }
    }

    pub fn nbytes(&self) -> usize {
        self.inner.dma_slice().size()
    }

    pub fn nsectors(&self) -> Sid {
        Sid::from_offset(self.nbytes())
    }

    pub fn nblocks(&self) -> usize {
        self.nbytes().align_up(BLOCK_SIZE) / BLOCK_SIZE
    }

    pub fn offset_within_first_block(&self) -> usize {
        self.inner.dma_slice().offset().start % BLOCK_SIZE
    }

    pub fn dma_slice(&self) -> &Slice<DmaBuffer<FromAndToDevice>> {
        self.inner.dma_slice()
    }
}

impl HasVmReaderWriter for BioSegment {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if self.inner.direction != BioDirection::FromDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.dma_slice().reader()
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if self.inner.direction != BioDirection::ToDevice {
            return Err(Error::AccessDenied);
        }
        self.inner.dma_slice().writer()
    }
}

impl BioSegmentInner {
    fn dma_slice(&self) -> &Slice<DmaBuffer<FromAndToDevice>> {
        &self.storage
    }
}

struct BioSegmentPool {
    arena_pool: Arc<DmaArenaPool<FromAndToDevice>>,
    direction: BioDirection,
}

impl BioSegmentPool {
    fn new(direction: BioDirection) -> Self {
        Self {
            arena_pool: DmaArenaPool::new(POOL_DEFAULT_NBLOCKS).unwrap(),
            direction,
        }
    }

    fn alloc(
        &self,
        nblocks: usize,
        offset_within_first_block: usize,
        len: usize,
    ) -> Option<BioSegmentInner> {
        assert!(
            offset_within_first_block < BLOCK_SIZE
                && offset_within_first_block + len <= nblocks * BLOCK_SIZE
        );

        let arena = self.arena_pool.alloc(nblocks)?;
        Some(BioSegmentInner {
            storage: Slice::new(
                DmaBuffer::Arena(arena),
                offset_within_first_block..offset_within_first_block + len,
            ),
            direction: self.direction,
        })
    }
}

static BIO_SEGMENT_RPOOL: Once<Arc<BioSegmentPool>> = Once::new();
static BIO_SEGMENT_WPOOL: Once<Arc<BioSegmentPool>> = Once::new();
const POOL_DEFAULT_NBLOCKS: usize = 4096;

pub fn bio_segment_pool_init() {
    BIO_SEGMENT_RPOOL.call_once(|| Arc::new(BioSegmentPool::new(BioDirection::FromDevice)));
    BIO_SEGMENT_WPOOL.call_once(|| Arc::new(BioSegmentPool::new(BioDirection::ToDevice)));
}

fn target_pool(direction: BioDirection) -> Option<&'static Arc<BioSegmentPool>> {
    match direction {
        BioDirection::FromDevice => BIO_SEGMENT_RPOOL.get(),
        BioDirection::ToDevice => BIO_SEGMENT_WPOOL.get(),
    }
}

pub(crate) fn is_sector_aligned(offset: usize) -> bool {
    offset.is_multiple_of(SECTOR_SIZE)
}
