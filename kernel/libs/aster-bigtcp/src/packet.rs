use alloc::{boxed::Box, sync::Arc};
use core::{marker::PhantomData, ops::Range};

use dma_pool::{DmaBuffer, DmaPool};
use ostd::{
    Result,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasSize, Infallible, PAGE_SIZE, USegment, VmReader,
        VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::HasVmReaderWriter,
    },
};
use ostd_pod::Pod;

pub trait Layer {
    type Preceding: Layer;

    const MAX_HEADER_SIZE: usize;

    const HEAD_ROOM_SIZE: usize =
        Self::Preceding::MAX_HEADER_SIZE + Self::Preceding::HEAD_ROOM_SIZE;
}

impl Layer for () {
    type Preceding = ();

    const MAX_HEADER_SIZE: usize = 0;
    const HEAD_ROOM_SIZE: usize = 0;
}

pub enum DeviceLayer {}
pub enum LinkLayer {}
pub enum NetworkLayer {}
pub enum TransportLayer {}
pub enum ApplicationLayer {}

impl Layer for DeviceLayer {
    type Preceding = ();
    const MAX_HEADER_SIZE: usize = 12;
}

impl Layer for LinkLayer {
    type Preceding = DeviceLayer;
    const MAX_HEADER_SIZE: usize = 14;
}

impl Layer for NetworkLayer {
    type Preceding = LinkLayer;
    const MAX_HEADER_SIZE: usize = 60;
}

impl Layer for TransportLayer {
    type Preceding = NetworkLayer;
    const MAX_HEADER_SIZE: usize = 60;
}

impl Layer for ApplicationLayer {
    type Preceding = TransportLayer;
    const MAX_HEADER_SIZE: usize = 0;
}

struct Common<D: DmaDirection> {
    segment: USegment,
    alloc_range: Range<usize>,
    data_range: Range<usize>,
    dma_buffer: Option<DmaBuffer<D>>,
}

impl<D: DmaDirection> Common<D> {
    fn range_to_sync(&self) -> Range<usize> {
        let offset = self.alloc_range.start;
        self.data_range.start - offset..self.data_range.end - offset
    }
}

type TxCommon = Common<ToDevice>;
type RxCommon = Common<FromDevice>;

pub struct FreshTxPacket<L = ApplicationLayer>(Box<TxCommon>, PhantomData<L>);

impl<L: Layer> FreshTxPacket<L> {
    pub fn alloc(payload_len: usize) -> Result<Self> {
        Self::alloc_inner(payload_len + L::HEAD_ROOM_SIZE, L::HEAD_ROOM_SIZE)
    }

    fn alloc_inner(total_len: usize, head_room_size: usize) -> Result<Self> {
        let nframes = total_len.div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new().alloc_segment(nframes)?.into();

        let common = Common {
            segment,
            alloc_range: 0..total_len,
            data_range: head_room_size..head_room_size,
            dma_buffer: None,
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    pub fn alloc_from_pool(pool: &Arc<DmaPool<ToDevice>>) -> Result<Self> {
        Self::alloc_from_pool_inner(L::HEAD_ROOM_SIZE, pool)
    }

    fn alloc_from_pool_inner(head_room_size: usize, pool: &Arc<DmaPool<ToDevice>>) -> Result<Self> {
        assert!(pool.segment_size() > head_room_size);

        let dma_segment = pool.alloc_segment()?;
        let (segment, alloc_range) = dma_segment.storage().into_parts();

        let common = Common {
            segment,
            alloc_range: alloc_range.clone(),
            data_range: alloc_range.start + head_room_size..alloc_range.start + head_room_size,
            dma_buffer: Some(DmaBuffer::Pooled(dma_segment)),
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    pub fn to_builder(self) -> TxPacketBuilder<L> {
        self.to_builder_at_layer()
    }

    pub fn to_builder_at_layer<H: Layer>(self) -> TxPacketBuilder<H> {
        const { assert!(L::HEAD_ROOM_SIZE >= H::HEAD_ROOM_SIZE) };

        TxPacketBuilder(self.0, PhantomData)
    }
}

pub struct TxPacketBuilder<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacketBuilder<L> {
    pub fn append_writer(&mut self) -> VmWriter<'_, Infallible> {
        let mut writer = self.0.segment.writer();
        writer
            .skip(self.0.data_range.end)
            .limit(self.0.alloc_range.end - self.0.data_range.end);
        writer
    }

    pub fn commit(&mut self, payload_len: usize) {
        assert!(payload_len <= self.0.alloc_range.end - self.0.data_range.end);
        self.0.data_range.end += payload_len;
    }

    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    pub fn build(self) -> TxPacket<L> {
        TxPacket(self.0, PhantomData)
    }
}

pub struct TxPacket<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacket<L> {
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.reader_with_header(0)
    }

    pub fn reader_with_header(&self, header_len: usize) -> VmReader<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut reader = self.0.segment.reader();
        reader
            .skip(self.0.data_range.start - header_len)
            .limit(self.0.data_range.end - self.0.data_range.start + header_len);
        reader
    }

    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    pub fn prepend_writer(&mut self, header_len: usize) -> VmWriter<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut writer = self.0.segment.writer();
        writer
            .skip(self.0.data_range.start - header_len)
            .limit(header_len);
        writer
    }

    pub fn reset_to_fresh(mut self) -> FreshTxPacket {
        const HEAD_ROOM_SIZE: usize = ApplicationLayer::HEAD_ROOM_SIZE;

        assert!(self.0.alloc_range.end - self.0.alloc_range.start >= HEAD_ROOM_SIZE);
        self.0.data_range.start = self.0.alloc_range.start + HEAD_ROOM_SIZE;
        self.0.data_range.end = self.0.alloc_range.start + HEAD_ROOM_SIZE;

        FreshTxPacket(self.0, PhantomData)
    }

    pub fn to_rx_packet(self) -> Option<RxPacket<L>> {
        if self.0.dma_buffer.is_some() {
            return None;
        }

        let common = Box::map(self.0, |common| Common {
            segment: common.segment,
            alloc_range: common.alloc_range,
            data_range: common.data_range,
            dma_buffer: None,
        });
        Some(RxPacket(common, PhantomData))
    }
}

impl<L: Layer> TxPacket<L> {
    pub fn prepend_and_pack<T: Pod>(mut self, header: &T) -> TxPacket<L::Preceding> {
        self.prepend_writer(size_of::<T>())
            .write_val(header)
            .unwrap();
        self.pack(size_of::<T>())
    }

    pub fn pack(mut self, header_len: usize) -> TxPacket<L::Preceding> {
        debug_assert!(header_len <= L::Preceding::MAX_HEADER_SIZE);

        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        self.0.data_range.start -= header_len;

        TxPacket(self.0, PhantomData)
    }
}

impl<L: Layer<Preceding = ()>> TxPacket<L> {
    pub fn map_dma(mut self, is_cache_coherent: bool) -> Result<TxBuffer> {
        if self.0.dma_buffer.is_none() {
            let dma_stream = DmaStream::<ToDevice>::map(self.0.segment.clone(), is_cache_coherent)?;
            self.0.dma_buffer = Some(DmaBuffer::Direct(dma_stream));
        }

        let tx_buffer = self.to_dma_buffer().unwrap();
        Ok(tx_buffer)
    }

    pub fn to_dma_buffer(self) -> Option<TxBuffer> {
        self.0
            .dma_buffer
            .as_ref()?
            .sync_to_device(self.0.range_to_sync())
            .unwrap();
        Some(TxBuffer(self.0))
    }
}

pub struct TxBuffer(Box<TxCommon>);

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        let offset = self.0.data_range.start - self.0.alloc_range.start;
        self.0.dma_buffer.as_ref().unwrap().daddr() + offset
    }
}

impl HasSize for TxBuffer {
    fn size(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }
}

pub struct RxBuffer(Box<RxCommon>);

impl RxBuffer {
    pub fn alloc(pool: &Arc<DmaPool<FromDevice>>) -> Result<Self> {
        let dma_segment = pool.alloc_segment()?;
        let (segment, alloc_range) = dma_segment.storage().into_parts();

        let common = Common {
            segment,
            alloc_range: alloc_range.clone(),
            data_range: alloc_range.start..alloc_range.start,
            dma_buffer: Some(DmaBuffer::Pooled(dma_segment)),
        };

        Ok(Self(Box::new(common)))
    }

    pub fn finish_dma(self, payload_len: usize) -> RxPacket<DeviceLayer> {
        self.finish_dma_at_layer(payload_len)
    }

    pub fn finish_dma_at_layer<L: Layer<Preceding = ()>>(
        mut self,
        payload_len: usize,
    ) -> RxPacket<L> {
        assert!(payload_len <= self.0.alloc_range.end - self.0.alloc_range.start);
        self.0.data_range.end = self.0.alloc_range.start + payload_len;

        debug_assert_eq!(self.0.data_range.start, self.0.alloc_range.start);

        self.0
            .dma_buffer
            .as_ref()
            .unwrap()
            .sync_from_device(self.0.range_to_sync())
            .unwrap();

        RxPacket(self.0, PhantomData)
    }
}

impl HasDaddr for RxBuffer {
    fn daddr(&self) -> Daddr {
        self.0.dma_buffer.as_ref().unwrap().daddr()
    }
}

impl HasSize for RxBuffer {
    fn size(&self) -> usize {
        self.0.alloc_range.end - self.0.alloc_range.start
    }
}

pub struct RxPacket<L>(Box<RxCommon>, PhantomData<L>);

impl<L> RxPacket<L> {
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.reader_with_header(0)
    }

    pub fn reader_with_header(&self, header_len: usize) -> VmReader<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut reader = self.0.segment.reader();
        reader
            .skip(self.0.data_range.start - header_len)
            .limit(self.0.data_range.end - self.0.data_range.start + header_len);
        reader
    }

    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    pub fn truncate(&mut self, len: usize) {
        assert!(len <= self.0.data_range.end - self.0.data_range.start);
        self.0.data_range.end = self.0.data_range.start + len;
    }
}

impl<L: Layer> RxPacket<L> {
    pub fn peel<H: Layer<Preceding = L>>(mut self, header_len: usize) -> RxPacket<H> {
        debug_assert!(header_len <= L::MAX_HEADER_SIZE);

        assert!(header_len <= self.0.data_range.end - self.0.data_range.start);
        self.0.data_range.start += header_len;

        RxPacket(self.0, PhantomData)
    }
}
