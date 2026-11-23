use core::ops::Range;

use lending_iterator::prelude::*;
use ostd_pod::{IntoBytes, Pod};

use super::{Iv, Key, Mac};
use crate::{
    layers::bio::{BlockLog, Buf},
    os::Aead,
    prelude::*,
};

pub(crate) struct CryptoChain<L> {
    block_log: L,
    key: Key,
    block_range: Range<BlockId>,
    block_macs: Vec<Mac>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct Footer {
    len: u32,
    pre_mac: Mac,
    this_mac: Mac,
    this_iv: Iv,
}

impl<L: BlockLog> CryptoChain<L> {
    pub(crate) const AVAIL_BLOCK_SIZE: usize = BLOCK_SIZE - size_of::<Footer>();

    pub(crate) fn new(block_log: L) -> Self {
        Self {
            block_log,
            key: Key::random(),
            block_range: 0..0,
            block_macs: Vec::new(),
        }
    }

    pub(crate) fn recover(key: Key, block_log: L, from: BlockId) -> Recovery<L> {
        Recovery::new(block_log, key, from)
    }

    pub(crate) fn read(&self, pos: BlockId, buf: &mut [u8]) -> Result<usize> {
        if !self.block_range().contains(&pos) {
            return_errno_with_msg!(NotFound, "read position is out of range");
        }

        let mut block_buf = Buf::alloc(1)?;
        self.block_log.read(pos, block_buf.as_mut())?;
        let footer: Footer = Pod::from_bytes(&block_buf.as_slice()[Self::AVAIL_BLOCK_SIZE..]);

        let payload_len = footer.len as usize;
        if payload_len > Self::AVAIL_BLOCK_SIZE || payload_len > buf.len() {
            return_errno_with_msg!(OutOfDisk, "wrong payload_len or the read_buf is too small");
        }

        let this_mac = self.block_macs.get(pos - self.block_range.start).unwrap();
        if footer.this_mac.as_bytes() != this_mac.as_bytes() {
            return_errno_with_msg!(NotFound, "check footer MAC failed");
        }

        let aead = Aead::new();
        aead.decrypt(
            &block_buf.as_slice()[..payload_len],
            self.key(),
            &footer.this_iv,
            &footer.pre_mac,
            &footer.this_mac,
            &mut buf[..payload_len],
        )?;
        Ok(payload_len)
    }

    pub(crate) fn append(&mut self, buf: &[u8]) -> Result<()> {
        if buf.len() > Self::AVAIL_BLOCK_SIZE {
            return_errno_with_msg!(OutOfDisk, "append data is too large");
        }
        let mut block_buf = Buf::alloc(1)?;

        let aead = Aead::new();
        let this_iv = Iv::random();
        let pre_mac = self.block_macs.last().copied().unwrap_or_default();
        let output = &mut block_buf.as_mut_slice()[..buf.len()];
        let this_mac = aead.encrypt(buf, self.key(), &this_iv, &pre_mac, output)?;

        let footer = Footer {
            len: buf.len() as _,
            pre_mac,
            this_mac,
            this_iv,
        };
        let buf = &mut block_buf.as_mut_slice()[Self::AVAIL_BLOCK_SIZE..];
        buf.copy_from_slice(footer.as_bytes());

        self.block_log.append(block_buf.as_ref())?;
        self.block_range.end += 1;
        self.block_macs.push(this_mac);
        Ok(())
    }

    pub(crate) fn flush(&self) -> Result<()> {
        self.block_log.flush()
    }

    pub(crate) fn trim(&mut self, before_block: BlockId) {
        debug_assert!(before_block < self.block_range.end);

        if before_block <= self.block_range.start {
            return;
        }

        let num_blocks_trimmed = before_block - self.block_range.start;
        self.block_range.start = before_block;
        self.block_macs.drain(..num_blocks_trimmed);
    }

    pub(crate) fn block_range(&self) -> &Range<BlockId> {
        &self.block_range
    }

    pub(crate) fn inner_log(&self) -> &L {
        &self.block_log
    }

    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
}

pub(crate) struct Recovery<L> {
    block_log: L,
    key: Key,
    block_range: Range<BlockId>,
    block_macs: Vec<Mac>,
    read_buf: Buf,
    payload: Buf,
}

impl<L: BlockLog> Recovery<L> {
    pub(crate) fn new(block_log: L, key: Key, first_block: BlockId) -> Self {
        Self {
            block_log,
            key,
            block_range: first_block..first_block,
            block_macs: Vec::new(),
            read_buf: Buf::alloc(1).unwrap(),
            payload: Buf::alloc(1).unwrap(),
        }
    }

    pub(crate) fn num_blocks(&self) -> usize {
        self.block_range.len()
    }

    pub(crate) fn block_range(&self) -> &Range<BlockId> {
        &self.block_range
    }

    pub(crate) fn block_macs(&self) -> &[Mac] {
        &self.block_macs
    }

    pub(crate) fn open(self) -> CryptoChain<L> {
        CryptoChain {
            block_log: self.block_log,
            key: self.key,
            block_range: self.block_range,
            block_macs: self.block_macs,
        }
    }
}

#[gat]
impl<L: BlockLog> LendingIterator for Recovery<L> {
    type Item<'a> = &'a [u8];

    fn next(&mut self) -> Option<Self::Item<'_>> {
        let next_block_id = self.block_range.end;
        self.block_log
            .read(next_block_id, self.read_buf.as_mut())
            .ok()?;

        let footer: Footer =
            Pod::from_bytes(&self.read_buf.as_slice()[CryptoChain::<L>::AVAIL_BLOCK_SIZE..]);
        let payload_len = footer.len as usize;
        if payload_len > CryptoChain::<L>::AVAIL_BLOCK_SIZE {
            return None;
        }

        let aead = Aead::new();
        aead.decrypt(
            &self.read_buf.as_slice()[..payload_len],
            &self.key,
            &footer.this_iv,
            &footer.pre_mac,
            &footer.this_mac,
            &mut self.payload.as_mut_slice()[..payload_len],
        )
        .ok()?;

        if self
            .block_macs()
            .last()
            .is_some_and(|mac| mac.as_bytes() != footer.pre_mac.as_bytes())
        {
            return None;
        }

        self.block_range.end += 1;
        self.block_macs.push(footer.this_mac);
        Some(&self.payload.as_slice()[..payload_len])
    }
}

#[cfg(test)]
mod tests {
    use lending_iterator::LendingIterator;

    use super::CryptoChain;
    use crate::layers::bio::{BlockLog, BlockRing, BlockSet, MemDisk};

    #[test]
    fn new() {
        let disk = MemDisk::create(16).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(0);
        let chain = CryptoChain::new(block_ring);

        assert_eq!(chain.block_log.nblocks(), 0);
        assert_eq!(chain.block_range.start, 0);
        assert_eq!(chain.block_range.end, 0);
        assert_eq!(chain.block_macs.len(), 0);
    }

    #[test]
    fn append_trim_and_read() {
        let disk = MemDisk::create(16).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(0);
        let mut chain = CryptoChain::new(block_ring);

        let data = [1u8; 1024];
        chain.append(&data[..256]).unwrap();
        chain.append(&data[..512]).unwrap();
        assert_eq!(chain.block_range.end, 2);
        assert_eq!(chain.block_macs.len(), 2);

        chain.trim(1);

        assert_eq!(chain.block_range.start, 1);
        assert_eq!(chain.block_range.end, 2);
        assert_eq!(chain.block_macs.len(), 1);

        let mut buf = [0u8; 1024];
        let len = chain.read(1, &mut buf).unwrap();
        assert_eq!(len, 512);
        assert_eq!(buf[..512], [1u8; 512]);
    }

    #[test]
    fn recover() {
        let disk = MemDisk::create(16).unwrap();
        let key = {
            let sub_disk = disk.subset(0..8).unwrap();
            let block_ring = BlockRing::new(sub_disk);
            block_ring.set_cursor(0);
            let data = [1u8; 1024];
            let mut chain = CryptoChain::new(block_ring);
            for _ in 0..4 {
                chain.append(&data).unwrap();
            }
            chain.flush().unwrap();
            chain.key
        };

        let sub_disk = disk.subset(0..8).unwrap();
        let block_ring = BlockRing::new(sub_disk);
        let mut recover = CryptoChain::recover(key, block_ring, 2);
        while let Some(payload) = recover.next() {
            assert_eq!(payload.len(), 1024);
        }
        let chain = recover.open();
        assert_eq!(chain.block_range(), &(2..4));
        assert_eq!(chain.block_macs.len(), 2);
    }
}
