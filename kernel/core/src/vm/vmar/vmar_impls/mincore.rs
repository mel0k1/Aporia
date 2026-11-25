//! The Vmar-level implementation of `mincore`.

use core::ops::Range;

use ostd::task::disable_preempt;

use super::Vmar;
use crate::prelude::*;

impl Vmar {
    /// Queries the residency of the pages in the specified range (`mincore`).
    ///
    /// For each page in `range`, the byte in `bits` at the matching position
    /// gets its least significant bit set if the page is currently present
    /// in the page tables.
    ///
    /// If some page in the range is not mapped, an [`ENOMEM`] error is
    /// returned. `bits` must contain one byte per page of the range.
    pub(crate) fn mincore_range(&self, range: Range<Vaddr>, bits: &mut [u8]) -> Result<()> {
        debug_assert_eq!(bits.len(), (range.end - range.start).div_ceil(PAGE_SIZE));

        let inner = self.inner.read();
        for (i, page) in (range.start..range.end).step_by(PAGE_SIZE).enumerate() {
            if inner.vm_mappings.find_one(&page).is_none() {
                return Err(errno!(ENOMEM, "the range is not fully mapped"));
            }

            let preempt_guard = disable_preempt();
            let mut cursor = self
                .vm_space
                .cursor(&preempt_guard, &(page..page + PAGE_SIZE))?;
            let is_present = matches!(cursor.query(), Ok((_, Some(_))));
            drop(cursor);
            drop(preempt_guard);

            if is_present {
                bits[i] |= 1;
            }
        }

        Ok(())
    }
}
