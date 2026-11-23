use core::ops::Range;

use ostd::mm::Vaddr;

pub(super) struct RelocatedRange {
    original_range: Range<Vaddr>,
    relocated_start: Vaddr,
}

impl RelocatedRange {
    pub(super) fn new(original_range: Range<Vaddr>, relocated_start: Vaddr) -> Option<Self> {
        relocated_start.checked_add(original_range.len())?;
        Some(Self {
            original_range,
            relocated_start,
        })
    }

    pub(super) fn relocated_addr_of(&self, addr: Vaddr) -> Option<Vaddr> {
        if self.original_range.contains(&addr) {
            Some(addr - self.original_range.start + self.relocated_start)
        } else {
            None
        }
    }

    pub(super) fn relocated_start(&self) -> Vaddr {
        self.relocated_start
    }
}
