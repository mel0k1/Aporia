use core::ops::Range;

use ostd::task::disable_preempt;

use super::{RssDelta, Vmar};
use crate::{
    prelude::*,
    vm::vmar::{VMAR_CAP_ADDR, interval_set::Interval, util::get_intersected_range},
};

impl Vmar {
    pub(super) fn clear(&self) {
        let mut inner = self.inner.write();
        for vm_mapping in inner.vm_mappings.iter() {
            if let Some(mut rmap) = vm_mapping.lock_rmap() {
                rmap.remove(self.weak_self.clone(), vm_mapping.map_to_addr());
            }
        }
        inner.vm_mappings.clear();

        let preempt_guard = disable_preempt();
        let full_range = 0..VMAR_CAP_ADDR;
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &full_range)
            .unwrap();
        cursor.unmap(full_range.len());
        cursor.flusher().sync_tlb_flush();
    }

    pub(crate) fn remove_mapping(&self, range: Range<usize>) {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);
        inner.alloc_free_region_exact_truncate(self, range.start, range.len(), &mut rss_delta);
    }

    pub(crate) fn discard_pages(&self, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let inner = self.inner.read();
        let mut rss_delta = RssDelta::new(self);

        let mut last_mapping_end = range.start;
        for vm_mapping in inner.vm_mappings.find(&range) {
            if !vm_mapping.can_expand() {
                return Err(errno!(EINVAL, "device mappings cannot be discarded"));
            }

            if last_mapping_end >= vm_mapping.map_to_addr() {
                last_mapping_end = vm_mapping.map_end();
            }

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let preempt_guard = disable_preempt();
            let vm_space = self.vm_space();
            let mut cursor = vm_space
                .cursor_mut(&preempt_guard, &intersected_range)
                .unwrap();

            rss_delta.add(
                vm_mapping.rss_type(),
                -(cursor.unmap(intersected_range.len()) as isize),
            );
            cursor.flusher().dispatch_tlb_flush();
            cursor.flusher().sync_tlb_flush();
        }

        if last_mapping_end < range.end {
            return Err(errno!(
                ENOMEM,
                "the range contains pages that are not mapped"
            ));
        }

        Ok(())
    }
}
