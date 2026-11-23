use spin::once::Once;

use crate::arch::iommu::{IrtEntryHandle, alloc_irt_entry, has_interrupt_remapping};

pub(crate) struct IrqRemapping {
    entry: Once<IrtEntryHandle>,
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { entry: Once::new() }
    }

    pub(crate) fn init(&self, irq_num: u8) {
        if !has_interrupt_remapping() {
            return;
        }

        self.entry.call_once(|| {
            let handle = alloc_irt_entry().unwrap();
            handle.enable(irq_num as u32);
            handle
        });
    }

    pub(crate) fn remapping_index(&self) -> Option<u16> {
        Some(self.entry.get()?.index())
    }
}
