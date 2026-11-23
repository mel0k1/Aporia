use alloc::vec;

use lending_iterator::prelude::*;

use crate::prelude::*;

pub struct Buf(Vec<u8>);

impl Buf {
    pub fn alloc(num_blocks: usize) -> Result<Self> {
        if num_blocks == 0 {
            return_errno_with_msg!(
                InvalidArgs,
                "num_blocks must be greater than 0 for allocation"
            )
        }
        let buffer = vec![0; num_blocks * BLOCK_SIZE];
        Ok(Self(buffer))
    }

    pub fn nblocks(&self) -> usize {
        self.0.len() / BLOCK_SIZE
    }

    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0.as_mut_slice()
    }

    pub fn as_ref(&self) -> BufRef<'_> {
        BufRef(self.as_slice())
    }

    pub fn as_mut(&mut self) -> BufMut<'_> {
        BufMut(self.as_mut_slice())
    }
}

#[derive(Clone, Copy)]
pub struct BufRef<'a>(&'a [u8]);

impl BufRef<'_> {
    pub fn as_slice(&self) -> &[u8] {
        self.0
    }

    pub fn nblocks(&self) -> usize {
        self.0.len() / BLOCK_SIZE
    }

    pub fn iter(&self) -> BufIter<'_> {
        BufIter {
            buf: BufRef(self.as_slice()),
            offset: 0,
        }
    }
}

impl<'a> TryFrom<&'a [u8]> for BufRef<'a> {
    type Error = Error;

    fn try_from(buf: &'a [u8]) -> Result<Self> {
        if buf.is_empty() {
            return_errno_with_msg!(InvalidArgs, "empty buf in `BufRef::try_from`");
        }
        if !buf.len().is_multiple_of(BLOCK_SIZE) {
            return_errno_with_msg!(
                NotBlockSizeAligned,
                "buf not block size aligned `BufRef::try_from`"
            );
        }

        let new_self = Self(buf);
        Ok(new_self)
    }
}

pub struct BufMut<'a>(&'a mut [u8]);

impl BufMut<'_> {
    pub fn as_slice(&self) -> &[u8] {
        self.0
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.0
    }

    pub fn nblocks(&self) -> usize {
        self.0.len() / BLOCK_SIZE
    }

    pub fn iter(&self) -> BufIter<'_> {
        BufIter {
            buf: BufRef(self.as_slice()),
            offset: 0,
        }
    }

    pub fn iter_mut(&mut self) -> BufIterMut<'_> {
        BufIterMut {
            buf: BufMut(self.as_mut_slice()),
            offset: 0,
        }
    }
}

impl<'a> TryFrom<&'a mut [u8]> for BufMut<'a> {
    type Error = Error;

    fn try_from(buf: &'a mut [u8]) -> Result<Self> {
        if buf.is_empty() {
            return_errno_with_msg!(InvalidArgs, "empty buf in `BufMut::try_from`");
        }
        if !buf.len().is_multiple_of(BLOCK_SIZE) {
            return_errno_with_msg!(
                NotBlockSizeAligned,
                "buf not block size aligned `BufMut::try_from`"
            );
        }

        let new_self = Self(buf);
        Ok(new_self)
    }
}

pub struct BufIter<'a> {
    buf: BufRef<'a>,
    offset: usize,
}

impl<'a> Iterator for BufIter<'a> {
    type Item = BufRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.buf.0.len() {
            return None;
        }

        let offset = self.offset;
        self.offset += BLOCK_SIZE;
        BufRef::try_from(&self.buf.0[offset..offset + BLOCK_SIZE]).ok()
    }
}

pub struct BufIterMut<'a> {
    buf: BufMut<'a>,
    offset: usize,
}

#[gat]
impl LendingIterator for BufIterMut<'_> {
    type Item<'next> = BufMut<'next>;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        if self.offset >= self.buf.0.len() {
            return None;
        }

        let offset = self.offset;
        self.offset += BLOCK_SIZE;
        BufMut::try_from(&mut self.buf.0[offset..offset + BLOCK_SIZE]).ok()
    }
}

#[cfg(test)]
mod tests {
    use lending_iterator::LendingIterator;

    use super::{BLOCK_SIZE, Buf, BufMut, BufRef};

    fn iterate_buf_ref<'a>(buf: BufRef<'a>) {
        for block in buf.iter() {
            assert_eq!(block.as_slice().len(), BLOCK_SIZE);
            assert_eq!(block.nblocks(), 1);
        }
    }

    fn iterate_buf_mut<'a>(mut buf: BufMut<'a>) {
        let mut iter_mut = buf.iter_mut();
        while let Some(mut block) = iter_mut.next() {
            assert_eq!(block.as_mut_slice().len(), BLOCK_SIZE);
            assert_eq!(block.nblocks(), 1);
        }
    }

    #[test]
    fn buf() {
        let mut buf = Buf::alloc(10).unwrap();
        assert_eq!(buf.nblocks(), 10);
        assert_eq!(buf.as_slice().len(), 10 * BLOCK_SIZE);
        iterate_buf_ref(buf.as_ref());
        iterate_buf_mut(buf.as_mut());

        let mut buf = [0u8; BLOCK_SIZE];
        iterate_buf_ref(BufRef::try_from(buf.as_slice()).unwrap());
        iterate_buf_mut(BufMut::try_from(buf.as_mut_slice()).unwrap());
    }
}
