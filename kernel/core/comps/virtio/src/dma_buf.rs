use alloc::sync::Arc;

use aster_bigtcp::packet::{RxBuffer, TxBuffer};
use aster_util::mem_obj_slice::Slice;
use dma_pool::{DmaBuffer, DmaSegment};
use ostd::mm::{
    HasDaddr, HasSize,
    dma::{DmaCoherent, DmaDirection, DmaStream},
};

pub(crate) trait DmaBuf: HasDaddr {
    fn len(&self) -> usize;
}

macro_rules! impl_dma_buf_for {
    (<D> $t:ty) => {
        impl<D: DmaDirection> DmaBuf for $t {
            fn len(&self) -> usize {
                self.size()
            }
        }

        impl<D: DmaDirection> DmaBuf for Slice<$t> {
            fn len(&self) -> usize {
                self.size()
            }
        }
    };
    ($t:ty) => {
        impl DmaBuf for $t {
            fn len(&self) -> usize {
                self.size()
            }
        }

        impl DmaBuf for Slice<$t> {
            fn len(&self) -> usize {
                self.size()
            }
        }
    };
}

impl_dma_buf_for!(<D> DmaStream<D>);
impl_dma_buf_for!(<D> &DmaStream<D>);
impl_dma_buf_for!(<D> Arc<DmaStream<D>>);
impl_dma_buf_for!(<D> &Arc<DmaStream<D>>);
impl_dma_buf_for!(DmaCoherent);
impl_dma_buf_for!(&DmaCoherent);
impl_dma_buf_for!(Arc<DmaCoherent>);
impl_dma_buf_for!(&Arc<DmaCoherent>);

impl<D: DmaDirection> DmaBuf for DmaSegment<D> {
    fn len(&self) -> usize {
        self.size()
    }
}

impl<D: DmaDirection> DmaBuf for Slice<DmaBuffer<D>> {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for TxBuffer {
    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for RxBuffer {
    fn len(&self) -> usize {
        self.size()
    }
}
