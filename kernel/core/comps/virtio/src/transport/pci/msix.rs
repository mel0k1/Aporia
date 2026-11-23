use alloc::vec::Vec;

use aster_pci::capability::msix::CapabilityMsixData;
use ostd::irq::IrqLine;

pub(super) struct VirtioMsixManager {
    config_msix_vector: u16,
    shared_interrupt_vector: u16,
    unused_msix_vectors: Vec<u16>,
    used_msix_vectors: Vec<u16>,
    msix: CapabilityMsixData,
}

impl VirtioMsixManager {
    pub(super) fn new(mut msix: CapabilityMsixData) -> Self {
        let mut msix_vector_list: Vec<u16> = (0..msix.table_size()).collect();
        for i in msix_vector_list.iter() {
            let irq = IrqLine::alloc().unwrap();
            msix.set_interrupt_vector(irq, *i);
        }
        let config_msix_vector = msix_vector_list.pop().unwrap();
        let shared_interrupt_vector = msix_vector_list.pop().unwrap();
        Self {
            config_msix_vector,
            shared_interrupt_vector,
            unused_msix_vectors: msix_vector_list,
            used_msix_vectors: Vec::new(),
            msix,
        }
    }

    pub(super) fn config_msix_irq(&mut self) -> (u16, &mut IrqLine) {
        (
            self.config_msix_vector,
            self.msix.irq_mut(self.config_msix_vector as usize).unwrap(),
        )
    }

    pub(super) fn shared_irq_line(&mut self) -> (u16, &mut IrqLine) {
        (
            self.shared_interrupt_vector,
            self.msix
                .irq_mut(self.shared_interrupt_vector as usize)
                .unwrap(),
        )
    }

    pub(super) fn pop_unused_irq(&mut self) -> Option<(u16, &mut IrqLine)> {
        let vector = self.unused_msix_vectors.pop()?;
        self.used_msix_vectors.push(vector);
        Some((vector, self.msix.irq_mut(vector as usize).unwrap()))
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.msix.is_enabled()
    }
}
