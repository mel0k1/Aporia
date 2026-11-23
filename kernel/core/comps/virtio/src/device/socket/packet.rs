use aster_bigtcp::packet;
use ostd::{
    Result,
    mm::{Infallible, VmReader, VmWriter},
};

use crate::device::socket::{
    buffer::{RX_BUFFER_POOL, TX_BUFFER_LEN, TX_BUFFER_POOL},
    header::VirtioVsockHdr,
};

pub struct TxPacket(packet::TxBuffer);

impl TxPacket {
    pub fn new(header: &VirtioVsockHdr) -> Result<Self> {
        Ok(Self::new_builder()?.build(header))
    }

    pub fn new_builder() -> Result<TxPacketBuilder> {
        let tx_pool = TX_BUFFER_POOL.get().unwrap();
        let inner = packet::FreshTxPacket::<PayloadLayer>::alloc_from_pool(tx_pool)?.to_builder();
        Ok(TxPacketBuilder(inner))
    }

    pub(super) fn inner(&self) -> &packet::TxBuffer {
        &self.0
    }
}

pub struct TxPacketBuilder(packet::TxPacketBuilder<PayloadLayer>);

impl TxPacketBuilder {
    pub const MAX_NBYTES: usize = TX_BUFFER_LEN - size_of::<VirtioVsockHdr>();

    pub fn copy_payload<F>(&mut self, copy_fn: F) -> Result<usize>
    where
        F: FnOnce(VmWriter<Infallible>) -> Result<usize>,
    {
        let writer = self.0.append_writer();
        let bytes_written = copy_fn(writer)?;
        self.0.commit(bytes_written);
        Ok(bytes_written)
    }

    pub fn payload_len(&self) -> usize {
        self.0.len()
    }

    pub fn build(self, header: &VirtioVsockHdr) -> TxPacket {
        let tx_packet = {
            let packet = self.0.build();
            packet.prepend_and_pack(header)
        };

        let tx_buffer = tx_packet.to_dma_buffer().unwrap();
        TxPacket(tx_buffer)
    }
}

pub struct RxPacket(packet::RxPacket<HeaderLayer>);

impl RxPacket {
    pub(super) fn new_builder() -> Result<RxPacketBuilder> {
        let inner = packet::RxBuffer::alloc(RX_BUFFER_POOL.get().unwrap())?;
        Ok(RxPacketBuilder(inner))
    }

    pub fn header(&self) -> VirtioVsockHdr {
        self.0.reader().read_val::<VirtioVsockHdr>().unwrap()
    }

    pub fn payload_len(&self) -> usize {
        self.0.len() - size_of::<VirtioVsockHdr>()
    }

    pub fn payload(&self) -> VmReader<'_, Infallible> {
        let mut reader = self.0.reader();
        reader.skip(size_of::<VirtioVsockHdr>());
        reader
    }
}

pub(super) struct RxPacketBuilder(packet::RxBuffer);

impl RxPacketBuilder {
    pub(super) fn inner(&self) -> &packet::RxBuffer {
        &self.0
    }

    pub(super) fn build(self, len: usize) -> RxPacket {
        RxPacket(self.0.finish_dma_at_layer(len))
    }
}

enum HeaderLayer {}
enum PayloadLayer {}

impl packet::Layer for HeaderLayer {
    type Preceding = ();
    const MAX_HEADER_SIZE: usize = size_of::<VirtioVsockHdr>();
}

impl packet::Layer for PayloadLayer {
    type Preceding = HeaderLayer;
    const MAX_HEADER_SIZE: usize = 0;
}
