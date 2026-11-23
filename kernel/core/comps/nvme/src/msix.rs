use alloc::vec::Vec;

use aster_pci::capability::msix::CapabilityMsixData;
use ostd::irq::IrqLine;

pub(crate) struct NvmeMsixManager {
    admin_vector: u16,
    unused_vectors: Vec<u16>,
    used_vectors: Vec<u16>,
    msix: CapabilityMsixData,
}

impl NvmeMsixManager {
    pub(crate) fn new(mut msix: CapabilityMsixData) -> Option<Self> {
        let table_size = msix.table_size();
        if table_size == 0 {
            return None;
        }
        let n = usize::from(table_size);

        let mut irqs = Vec::with_capacity(n);
        for _ in 0..n {
            irqs.push(IrqLine::alloc().ok()?);
        }

        for (i, irq) in irqs.into_iter().enumerate() {
            msix.set_interrupt_vector(irq, i as u16);
        }

        let mut vector_list: Vec<u16> = (0..table_size).collect();
        let admin_vector = vector_list.remove(0);

        Some(Self {
            admin_vector,
            unused_vectors: vector_list,
            used_vectors: Vec::new(),
            msix,
        })
    }

    pub(crate) fn admin_irq(&mut self) -> (u16, &mut IrqLine) {
        (
            self.admin_vector,
            self.msix.irq_mut(self.admin_vector as usize).unwrap(),
        )
    }

    pub(crate) fn alloc_io_queue_irq(&mut self) -> Option<(u16, &mut IrqLine)> {
        let vector = self.unused_vectors.pop()?;
        self.used_vectors.push(vector);
        Some((vector, self.msix.irq_mut(vector as usize).unwrap()))
    }

    pub(crate) fn table_size(&self) -> u16 {
        self.msix.table_size()
    }

    pub(crate) fn irq_for_vector_mut(&mut self, vector: u16) -> Option<&mut IrqLine> {
        self.msix.irq_mut(vector as usize)
    }
}
