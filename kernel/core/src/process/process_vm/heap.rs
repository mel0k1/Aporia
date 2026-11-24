use core::ops::Range;

use align_ext::AlignExt;

use crate::{
    prelude::*,
    process::ResourceType,
    util::random::getrandom,
    vm::{
        perms::VmPerms,
        vmar::{VMAR_CAP_ADDR, Vmar, VmarMapOffset},
    },
};

#[derive(Debug)]
pub(crate) struct Heap {
    inner: Mutex<Option<HeapInner>>,
}

#[derive(Debug)]
pub(crate) struct LockedHeap<'a> {
    inner: MutexGuard<'a, Option<HeapInner>>,
}

#[derive(Clone, Debug)]
struct HeapInner {
    /// The size of the data segment, used for rlimit checking.
    data_segment_size: usize,
    /// The heap range.
    heap_range: Range<Vaddr>,
}

impl Heap {
    pub(super) const fn new_uninitialized() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Creates a new `Heap` with identical contents of an existing one.
    pub(super) fn fork_from(heap_guard: &LockedHeap) -> Self {
        let inner = heap_guard.inner.as_ref().expect("Heap is not initialized");

        Self {
            inner: Mutex::new(Some(inner.clone())),
        }
    }

    /// Initializes and maps the heap virtual memory.
    pub(super) fn map_and_init_heap(
        &self,
        vmar: &Vmar,
        data_segment_size: usize,
        heap_base: Vaddr,
    ) -> Result<()> {
        let mut inner = self.inner.lock();

        let nr_pages_padding = {
            let mut nr_random_padding_pages: u8 = 0;
            getrandom(nr_random_padding_pages.as_mut_bytes());

            nr_random_padding_pages as usize
        };

        let heap_start = heap_base.align_up(PAGE_SIZE) + nr_pages_padding * PAGE_SIZE;
        let heap_end = heap_start + PAGE_SIZE;
        if heap_end > VMAR_CAP_ADDR {
            return Err(errno!(ENOMEM, "the mapping address is too large"));
        }

        let vmar_map_options = {
            let perms = VmPerms::READ | VmPerms::WRITE;
            vmar.new_map(PAGE_SIZE, perms)
                .offset(VmarMapOffset::FixedNoReplace(heap_start))
        };
        vmar_map_options.build()?;

        debug_assert!(inner.is_none());
        *inner = Some(HeapInner {
            data_segment_size,
            heap_range: heap_start..heap_end,
        });

        Ok(())
    }

    /// Locks the heap and returns a guard to access the heap information.
    pub(crate) fn lock(&self) -> LockedHeap<'_> {
        LockedHeap {
            inner: self.inner.lock(),
        }
    }

    /// Modifies the end address of the heap.
    ///
    /// Returns the new heap end on success, or the current heap end on failure.
    /// This behavior is consistent with the Linux `brk` syscall.
    pub(crate) fn modify_heap_end(
        &self,
        new_heap_end: Vaddr,
        ctx: &Context,
    ) -> Result<Vaddr, Vaddr> {
        let user_space = ctx.user_space();
        let vmar = user_space.vmar();

        let mut inner = self.inner.lock();
        let inner = inner.as_mut().expect("Heap is not initialized");

        let heap_start = inner.heap_range.start;
        let current_heap_end = inner.heap_range.end;
        let new_heap_range = heap_start..new_heap_end;

        if new_heap_end < heap_start
            || new_heap_end > VMAR_CAP_ADDR
            || check_data_rlimit(inner.data_segment_size, &new_heap_range, ctx).is_err()
        {
            return Err(current_heap_end);
        }

        let current_heap_end_aligned = current_heap_end.align_up(PAGE_SIZE);
        let new_heap_end_aligned = new_heap_end.align_up(PAGE_SIZE);

        let old_size = current_heap_end_aligned - heap_start;
        let new_size = new_heap_end_aligned - heap_start;

        if old_size == new_size {
            inner.heap_range = new_heap_range;
            return Ok(new_heap_end);
        }

        if new_size < old_size {
            vmar.remove_mapping(new_heap_end_aligned..current_heap_end_aligned);
        } else {
            let expansion_size = new_size - old_size;
            let expansion_start = heap_start + old_size;

            let vmar_map_options = {
                let perms = VmPerms::READ | VmPerms::WRITE;
                vmar.new_map(expansion_size, perms)
                    .offset(VmarMapOffset::FixedNoReplace(expansion_start))
            };
            vmar_map_options.build().map_err(|_| current_heap_end)?;
        }

        inner.heap_range = new_heap_range;
        Ok(new_heap_end)
    }
}

impl LockedHeap<'_> {
    /// Returns the lowest address of the heap.
    pub(crate) fn heap_low(&self) -> Vaddr {
        self.heap_range().start
    }

    /// Returns the current heap range.
    pub(crate) fn heap_range(&self) -> &Range<Vaddr> {
        let inner = self.inner.as_ref().expect("Heap is not initialized");

        &inner.heap_range
    }
}

/// Checks whether the new heap range exceeds the data segment size limit.
fn check_data_rlimit(
    data_segment_size: usize,
    new_heap_range: &Range<Vaddr>,
    ctx: &Context,
) -> Result<()> {
    let rlimit_data = ctx
        .process
        .resource_limits()
        .get_rlimit(ResourceType::RLIMIT_DATA)
        .get_cur();

    if rlimit_data.saturating_sub(data_segment_size as u64) < new_heap_range.len() as u64 {
        return Err(errno!(ENOSPC, "the data segment size limit is reached"));
    }
    Ok(())
}
