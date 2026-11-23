use super::{BlockLog, BlockSet, BufMut, BufRef};
use crate::{os::Mutex, prelude::*};

pub(crate) struct BlockRing<S> {
    storage: S,
    cursor: Mutex<Option<BlockId>>,
}

impl<S: BlockSet> BlockRing<S> {
    pub(crate) fn new(storage: S) -> Self {
        Self {
            storage,
            cursor: Mutex::new(None),
        }
    }

    pub(crate) fn set_cursor(&self, new_cursor: BlockId) {
        *self.cursor.lock() = Some(new_cursor);
    }

    pub(crate) fn storage(&self) -> &S {
        &self.storage
    }
}

impl<S: BlockSet> BlockLog for BlockRing<S> {
    fn read(&self, pos: BlockId, buf: BufMut) -> Result<()> {
        let pos = pos % self.storage.nblocks();
        self.storage.read(pos, buf)
    }

    fn append(&self, buf: BufRef) -> Result<BlockId> {
        let cursor = self
            .cursor
            .lock()
            .expect("cursor must be set before appending new blocks");
        let pos = cursor % self.storage.nblocks();
        let new_cursor = cursor + buf.nblocks();
        self.storage.write(pos, buf)?;
        self.set_cursor(new_cursor);
        Ok(cursor)
    }

    fn flush(&self) -> Result<()> {
        self.storage.flush()
    }

    fn nblocks(&self) -> usize {
        self.cursor.lock().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::BlockRing;
    use crate::layers::bio::{BlockLog, Buf, MemDisk};

    #[test]
    fn block_ring() {
        let num_blocks = 16;
        let disk = MemDisk::create(num_blocks).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(num_blocks);
        assert_eq!(block_ring.nblocks(), num_blocks);

        let mut append_buf = Buf::alloc(1).unwrap();
        append_buf.as_mut_slice().fill(1);
        let pos = block_ring.append(append_buf.as_ref()).unwrap();
        assert_eq!(pos, num_blocks);
        assert_eq!(block_ring.nblocks(), num_blocks + 1);

        let mut read_buf = Buf::alloc(1).unwrap();
        block_ring
            .read(pos % num_blocks, read_buf.as_mut())
            .unwrap();
        assert_eq!(read_buf.as_slice(), append_buf.as_slice());
    }
}
