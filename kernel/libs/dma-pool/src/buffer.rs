use core::ops::Range;

use ostd::{
    Result,
    mm::{
        Daddr, HasDaddr, HasSize, Infallible, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
};

use crate::{DmaArena, DmaSegment};

#[derive(Debug)]
pub enum DmaBuffer<D: DmaDirection> {
    Direct(DmaStream<D>),
    Pooled(DmaSegment<D>),
    Arena(DmaArena<D>),
}

impl<D: DmaDirection> DmaBuffer<D> {
    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Direct(stream) => stream.sync_from_device(byte_range),
            Self::Pooled(segment) => segment.sync_from_device(byte_range),
            Self::Arena(arena) => arena.sync_from_device(byte_range),
        }
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        match self {
            Self::Direct(stream) => stream.sync_to_device(byte_range),
            Self::Pooled(segment) => segment.sync_to_device(byte_range),
            Self::Arena(arena) => arena.sync_to_device(byte_range),
        }
    }
}

impl<D: DmaDirection> HasDaddr for DmaBuffer<D> {
    fn daddr(&self) -> Daddr {
        match self {
            Self::Direct(stream) => stream.daddr(),
            Self::Pooled(segment) => segment.daddr(),
            Self::Arena(arena) => arena.daddr(),
        }
    }
}

impl<D: DmaDirection> HasSize for DmaBuffer<D> {
    fn size(&self) -> usize {
        match self {
            Self::Direct(stream) => stream.size(),
            Self::Pooled(segment) => segment.size(),
            Self::Arena(arena) => arena.size(),
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaBuffer<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        match self {
            Self::Direct(stream) => stream.reader(),
            Self::Pooled(segment) => segment.reader(),
            Self::Arena(arena) => arena.reader(),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        match self {
            Self::Direct(stream) => stream.writer(),
            Self::Pooled(segment) => segment.writer(),
            Self::Arena(arena) => arena.writer(),
        }
    }
}
