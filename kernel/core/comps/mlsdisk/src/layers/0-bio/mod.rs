mod block_buf;
mod block_log;
mod block_ring;
mod block_set;

use ostd::const_assert;

pub use self::{
    block_buf::{Buf, BufMut, BufRef},
    block_set::BlockSet,
};
pub(crate) use self::{
    block_log::{BlockLog, MemLog},
    block_ring::BlockRing,
};

pub type BlockId = usize;
pub const BLOCK_SIZE: usize = 0x1000;
pub(crate) const BID_SIZE: usize = size_of::<BlockId>();

const_assert!(BID_SIZE == 8);
