use serde::{Deserialize, Serialize};

use crate::{
    layers::edit::Edit,
    os::{HashMap, Mutex},
    prelude::*,
    tx::{CurrentTx, TxData, TxProvider},
    util::BitMap,
};

pub(super) type ChunkId = usize;

pub(super) const CHUNK_NBLOCKS: usize = 1024;
pub(super) const CHUNK_SIZE: usize = CHUNK_NBLOCKS * BLOCK_SIZE;

#[derive(Clone)]
pub(super) struct ChunkAlloc {
    state: Arc<Mutex<ChunkAllocState>>,
    tx_provider: Arc<TxProvider>,
}

impl ChunkAlloc {
    pub(super) fn new(capacity: usize, tx_provider: Arc<TxProvider>) -> Self {
        let state = ChunkAllocState::new(capacity);
        Self::from_parts(state, tx_provider)
    }

    pub(super) fn from_parts(mut state: ChunkAllocState, tx_provider: Arc<TxProvider>) -> Self {
        state.in_journal = false;
        let new_self = Self {
            state: Arc::new(Mutex::new(state)),
            tx_provider,
        };

        new_self
            .tx_provider
            .register_data_initializer(Box::new(ChunkAllocEdit::new));

        new_self.tx_provider.register_commit_handler({
            let state = new_self.state.clone();
            move |current: CurrentTx<'_>| {
                let state = state.clone();
                current.data_with(move |edit: &ChunkAllocEdit| {
                    if edit.edit_table.is_empty() {
                        return;
                    }

                    let mut state = state.lock();
                    edit.apply_to(&mut state);
                });
            }
        });

        new_self.tx_provider.register_abort_handler({
            let state = new_self.state.clone();
            move |current: CurrentTx<'_>| {
                let state = state.clone();
                current.data_with(move |edit: &ChunkAllocEdit| {
                    let mut state = state.lock();
                    for chunk_id in edit.iter_allocated_chunks() {
                        state.dealloc(chunk_id);
                    }
                });
            }
        });

        new_self
    }

    pub(super) fn new_tx(&self) -> CurrentTx<'_> {
        self.tx_provider.new_tx()
    }

    pub(super) fn alloc(&self) -> Option<ChunkId> {
        let chunk_id = {
            let mut state = self.state.lock();
            state.alloc()?
        };

        let mut current_tx = self.tx_provider.current();
        current_tx.data_mut_with(|edit: &mut ChunkAllocEdit| {
            edit.alloc(chunk_id);
        });

        Some(chunk_id)
    }

    pub(super) fn alloc_batch(&self, count: usize) -> Option<Vec<ChunkId>> {
        let chunk_ids = {
            let mut ids = Vec::with_capacity(count);
            let mut state = self.state.lock();
            for _ in 0..count {
                match state.alloc() {
                    Some(id) => ids.push(id),
                    None => {
                        ids.iter().for_each(|id| state.dealloc(*id));
                        return None;
                    }
                }
            }
            ids.sort_unstable();
            ids
        };

        let mut current_tx = self.tx_provider.current();
        current_tx.data_mut_with(|edit: &mut ChunkAllocEdit| {
            for chunk_id in &chunk_ids {
                edit.alloc(*chunk_id);
            }
        });

        Some(chunk_ids)
    }

    pub(super) fn dealloc(&self, chunk_id: ChunkId) {
        let mut current_tx = self.tx_provider.current();
        current_tx.data_mut_with(|edit: &mut ChunkAllocEdit| {
            let should_dealloc_now = edit.dealloc(chunk_id);

            if should_dealloc_now {
                let mut state = self.state.lock();
                state.dealloc(chunk_id);
            }
        });
    }

    pub(super) fn dealloc_batch<I>(&self, chunk_ids: I)
    where
        I: Iterator<Item = ChunkId>,
    {
        let mut current_tx = self.tx_provider.current();
        current_tx.data_mut_with(|edit: &mut ChunkAllocEdit| {
            let mut state = self.state.lock();
            for chunk_id in chunk_ids {
                let should_dealloc_now = edit.dealloc(chunk_id);

                if should_dealloc_now {
                    state.dealloc(chunk_id);
                }
            }
        });
    }

    pub(super) fn capacity(&self) -> usize {
        self.state.lock().capacity()
    }

    pub(super) fn free_count(&self) -> usize {
        self.state.lock().free_count()
    }
}

impl Debug for ChunkAlloc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock();
        f.debug_struct("ChunkAlloc")
            .field("bitmap_free_count", &state.free_count)
            .field("bitmap_next_free", &state.next_free)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ChunkAllocState {
    alloc_map: BitMap,
    free_count: usize,
    next_free: usize,
    in_journal: bool,
}

impl ChunkAllocState {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            alloc_map: BitMap::repeat(false, capacity),
            free_count: capacity,
            next_free: 0,
            in_journal: false,
        }
    }

    pub(super) fn new_in_journal(capacity: usize) -> Self {
        Self {
            alloc_map: BitMap::repeat(false, capacity),
            free_count: capacity,
            next_free: 0,
            in_journal: true,
        }
    }

    pub(super) fn alloc(&mut self) -> Option<ChunkId> {
        let mut next_free = self.next_free;
        if next_free == self.alloc_map.len() {
            next_free = 0;
        }

        let free_chunk_id = {
            if let Some(chunk_id) = self.alloc_map.first_zero(next_free) {
                chunk_id
            } else {
                self.alloc_map
                    .first_zero(0)
                    .expect("there must exists a zero")
            }
        };

        self.alloc_map.set(free_chunk_id, true);
        self.free_count -= 1;
        self.next_free = free_chunk_id + 1;

        Some(free_chunk_id)
    }

    pub(super) fn dealloc(&mut self, chunk_id: ChunkId) {
        debug_assert!(self.alloc_map[chunk_id]);
        self.alloc_map.set(chunk_id, false);
        self.free_count += 1;
    }

    pub(super) fn capacity(&self) -> usize {
        self.alloc_map.len()
    }

    pub(super) fn free_count(&self) -> usize {
        self.free_count
    }

    pub(super) fn is_chunk_allocated(&self, chunk_id: ChunkId) -> bool {
        self.alloc_map[chunk_id]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ChunkAllocEdit {
    edit_table: HashMap<ChunkId, ChunkEdit>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum ChunkEdit {
    Alloc,
    Dealloc,
}

impl ChunkAllocEdit {
    pub(super) fn new() -> Self {
        Self {
            edit_table: HashMap::new(),
        }
    }

    pub(super) fn alloc(&mut self, chunk_id: ChunkId) {
        let old_edit = self.edit_table.insert(chunk_id, ChunkEdit::Alloc);

        assert!(old_edit.is_none());
    }

    pub(super) fn dealloc(&mut self, chunk_id: ChunkId) -> bool {
        match self.edit_table.get(&chunk_id) {
            None => {
                self.edit_table.insert(chunk_id, ChunkEdit::Dealloc);
                false
            }
            Some(&ChunkEdit::Alloc) => {
                self.edit_table.remove(&chunk_id);
                true
            }
            Some(&ChunkEdit::Dealloc) => {
                panic!("a chunk must not be deallocated twice");
            }
        }
    }

    pub(super) fn iter_allocated_chunks(&self) -> impl Iterator<Item = ChunkId> + '_ {
        self.edit_table.iter().filter_map(|(id, edit)| {
            if *edit == ChunkEdit::Alloc {
                Some(*id)
            } else {
                None
            }
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.edit_table.is_empty()
    }
}

impl Edit<ChunkAllocState> for ChunkAllocEdit {
    fn apply_to(&self, state: &mut ChunkAllocState) {
        let mut to_be_deallocated = Vec::new();
        for (&chunk_id, chunk_edit) in &self.edit_table {
            match chunk_edit {
                ChunkEdit::Alloc => {
                    if state.in_journal {
                        let _allocated_id = state.alloc().unwrap();
                    }
                }
                ChunkEdit::Dealloc => {
                    to_be_deallocated.push(chunk_id);
                }
            }
        }
        for chunk_id in to_be_deallocated {
            state.dealloc(chunk_id);
        }
    }
}

impl TxData for ChunkAllocEdit {}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_chunk_alloc() -> ChunkAlloc {
        let cap = 1024_usize;
        let tx_provider = TxProvider::new();
        let chunk_alloc = ChunkAlloc::new(cap, tx_provider);
        assert_eq!(chunk_alloc.capacity(), cap);
        assert_eq!(chunk_alloc.free_count(), cap);
        chunk_alloc
    }

    fn do_alloc_dealloc_tx(chunk_alloc: &ChunkAlloc, alloc_cnt: usize, dealloc_cnt: usize) -> Tx {
        debug_assert!(alloc_cnt <= chunk_alloc.capacity() && dealloc_cnt <= alloc_cnt);
        let mut tx = chunk_alloc.new_tx();
        tx.context(|| {
            let chunk_id = chunk_alloc.alloc().unwrap();
            let chunk_ids = chunk_alloc.alloc_batch(alloc_cnt - 1).unwrap();
            let allocated_chunk_ids: Vec<ChunkId> = core::iter::once(chunk_id)
                .chain(chunk_ids.into_iter())
                .collect();

            chunk_alloc.dealloc(allocated_chunk_ids[0]);
            chunk_alloc.dealloc_batch(
                allocated_chunk_ids[alloc_cnt - dealloc_cnt + 1..alloc_cnt]
                    .iter()
                    .cloned(),
            );
        });
        tx
    }

    #[test]
    fn chunk_alloc_dealloc_tx_commit() -> Result<()> {
        let chunk_alloc = new_chunk_alloc();
        let cap = chunk_alloc.capacity();
        let (alloc_cnt, dealloc_cnt) = (cap, cap);

        let mut tx = do_alloc_dealloc_tx(&chunk_alloc, alloc_cnt, dealloc_cnt);
        tx.commit()?;
        assert_eq!(chunk_alloc.free_count(), cap - alloc_cnt + dealloc_cnt);
        Ok(())
    }

    #[test]
    fn chunk_alloc_dealloc_tx_abort() -> Result<()> {
        let chunk_alloc = new_chunk_alloc();
        let cap = chunk_alloc.capacity();
        let (alloc_cnt, dealloc_cnt) = (cap / 2, cap / 4);

        let mut tx = do_alloc_dealloc_tx(&chunk_alloc, alloc_cnt, dealloc_cnt);
        tx.abort();
        assert_eq!(chunk_alloc.free_count(), cap);
        Ok(())
    }
}
