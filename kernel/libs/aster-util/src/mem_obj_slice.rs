use alloc::sync::Arc;
use core::{borrow::Borrow, fmt::Debug, ops::Range};

use ostd::mm::{
    HasDaddr, HasPaddr, HasSize, Infallible, VmReader, VmWriter,
    dma::DmaStream,
    io::util::{HasVmReaderWriter, VmReaderWriterResult},
};

macro_rules! assert_in_range {
    ($range:expr, $size:expr) => {
        assert!(!$range.is_empty(), "The range must not be empty");
        assert!($range.end <= $size, "The range end must be within the size");
    };
}

#[derive(Debug)]
pub struct Slice<MemObj> {
    inner: MemObj,
    offset: Range<usize>,
}

impl<MemObj: HasSize> HasSize for Slice<MemObj> {
    fn size(&self) -> usize {
        self.offset.end - self.offset.start
    }
}

impl<MemObj: HasSize + HasPaddr> HasPaddr for Slice<MemObj> {
    fn paddr(&self) -> ostd::mm::Paddr {
        self.mem_obj().paddr() + self.offset().start
    }
}

impl<MemObj: HasSize + HasDaddr> HasDaddr for Slice<MemObj> {
    fn daddr(&self) -> ostd::mm::Daddr {
        self.mem_obj().daddr() + self.offset().start
    }
}

impl<MemObj: HasSize> Slice<MemObj> {
    pub fn new(mem_obj: MemObj, offset: Range<usize>) -> Self {
        assert_in_range!(offset, mem_obj.size());
        Self {
            inner: mem_obj,
            offset,
        }
    }

    pub fn mem_obj(&self) -> &MemObj {
        &self.inner
    }

    pub fn offset(&self) -> &Range<usize> {
        &self.offset
    }

    pub fn into_parts(self) -> (MemObj, Range<usize>) {
        (self.inner, self.offset)
    }
}

impl<MemObj: HasSize + Clone> Slice<MemObj> {
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert_in_range!(range, self.size());
        let adj_range = (self.offset().start + range.start)..(self.offset().start + range.end);
        Self::new(self.mem_obj().clone(), adj_range)
    }
}

impl<MemObj: HasSize + HasVmReaderWriter<Types = VmReaderWriterResult>> HasVmReaderWriter
    for Slice<MemObj>
{
    type Types = VmReaderWriterResult;

    fn reader(&self) -> ostd::prelude::Result<VmReader<'_, Infallible>> {
        let mut reader = self.mem_obj().reader()?;
        reader.skip(self.offset().start).limit(self.size());
        Ok(reader)
    }

    fn writer(&self) -> ostd::prelude::Result<VmWriter<'_, Infallible>> {
        let mut writer = self.mem_obj().writer()?;
        writer.skip(self.offset().start).limit(self.size());
        Ok(writer)
    }
}

impl<MemObj: HasSize + Borrow<Arc<DmaStream>>> Slice<MemObj> {
    pub fn sync_from_device(&self) -> ostd::prelude::Result<()> {
        self.mem_obj()
            .borrow()
            .sync_from_device(self.offset().clone())
    }

    pub fn sync_to_device(&self) -> ostd::prelude::Result<()> {
        self.mem_obj()
            .borrow()
            .sync_to_device(self.offset().clone())
    }
}
