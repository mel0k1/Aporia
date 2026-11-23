use core::ops::RangeInclusive;

use super::mlsdisk::RecordKey;
use crate::{
    layers::bio::{BufMut, BufRef},
    os::{BTreeMap, Condvar, CvarMutex, Mutex},
    prelude::*,
};

#[derive(Debug)]
pub(super) struct DataBuf {
    buf: Mutex<BTreeMap<RecordKey, Arc<DataBlock>>>,
    cap: usize,
    cvar: Condvar,
    is_full: CvarMutex<bool>,
}

pub(super) struct DataBlock([u8; BLOCK_SIZE]);

impl DataBuf {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            buf: Mutex::new(BTreeMap::new()),
            cap,
            cvar: Condvar::new(),
            is_full: CvarMutex::new(false),
        }
    }

    pub(super) fn get(&self, key: RecordKey, buf: &mut BufMut) -> Option<()> {
        debug_assert_eq!(buf.nblocks(), 1);
        if let Some(block) = self.buf.lock().get(&key) {
            buf.as_mut_slice().copy_from_slice(block.as_slice());
            Some(())
        } else {
            None
        }
    }

    pub(super) fn get_range(
        &self,
        range: RangeInclusive<RecordKey>,
    ) -> Vec<(RecordKey, Arc<DataBlock>)> {
        self.buf
            .lock()
            .iter()
            .filter_map(|(k, v)| {
                if range.contains(k) {
                    Some((*k, v.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    pub(super) fn put(&self, key: RecordKey, buf: BufRef) -> bool {
        debug_assert_eq!(buf.nblocks(), 1);

        let mut is_full = self.is_full.lock().unwrap();
        while *is_full {
            is_full = self.cvar.wait(is_full).unwrap();
        }
        debug_assert!(!*is_full);

        let mut data_buf = self.buf.lock();
        let _ = data_buf.insert(key, DataBlock::from_buf(buf));

        if data_buf.len() >= self.cap {
            *is_full = true;
        }
        *is_full
    }

    pub(super) fn nblocks(&self) -> usize {
        self.buf.lock().len()
    }

    pub(super) fn at_capacity(&self) -> bool {
        self.nblocks() >= self.cap
    }

    pub(super) fn is_empty(&self) -> bool {
        self.nblocks() == 0
    }

    pub(super) fn clear(&self) {
        let mut is_full = self.is_full.lock().unwrap();
        self.buf.lock().clear();
        if *is_full {
            *is_full = false;
            self.cvar.notify_all();
        }
    }

    pub(super) fn all_blocks(&self) -> Vec<(RecordKey, Arc<DataBlock>)> {
        self.buf
            .lock()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }
}

impl DataBlock {
    pub(super) fn from_buf(buf: BufRef) -> Arc<Self> {
        debug_assert_eq!(buf.nblocks(), 1);
        Arc::new(DataBlock(buf.as_slice().try_into().unwrap()))
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Debug for DataBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataBlock")
            .field("first 16 bytes", &&self.0[..16])
            .finish()
    }
}
