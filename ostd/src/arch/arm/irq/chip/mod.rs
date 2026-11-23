use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use gicv3::Gic;
use spin::Once;

use super::HwIrqLine;
use crate::{Result, arch::boot::DEVICE_TREE, io::IoMemAllocatorBuilder, irq::IrqLine};

mod gicv3;

pub static IRQ_CHIP: Once<IrqChip> = Once::new();

pub(in crate::arch) fn init(io_mem_allocator_builder: &mut IoMemAllocatorBuilder) {
    let fdt = DEVICE_TREE.get().unwrap();
    let Some(gic) = Gic::from_fdt(fdt, io_mem_allocator_builder) else {
        crate::error!("No supported GICs found (only GICv3 is supported now)");
        return;
    };
    crate::info!("Found and initialized GICv3");

    IRQ_CHIP.call_once(|| IrqChip { gic });
}

pub struct IrqChip {
    gic: Gic,
}

impl IrqChip {
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let interrupt_source_on_chip = self
            .gic
            .map_interrupt_source_to(interrupt_source_in_fdt, &irq_line)?;
        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip,
        })
    }

    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        self.gic
            .unmap_interrupt_source(mapped_irq_line.interrupt_source_on_chip);
    }

    pub(in crate::arch) fn claim_interrupt(&self) -> Option<HwIrqLine> {
        self.gic.claim_interrupt()
    }

    pub(super) fn complete_interrupt(&self, interrupt_source: InterruptSourceOnChip) {
        self.gic.complete_interrupt(interrupt_source);
    }
}

pub struct MappedIrqLine {
    irq_line: IrqLine,
    interrupt_source_on_chip: InterruptSourceOnChip,
}

impl fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("interrupt_source_on_chip", &self.interrupt_source_on_chip)
            .finish_non_exhaustive()
    }
}

impl Deref for MappedIrqLine {
    type Target = IrqLine;

    fn deref(&self) -> &Self::Target {
        &self.irq_line
    }
}

impl DerefMut for MappedIrqLine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.irq_line
    }
}

impl Drop for MappedIrqLine {
    fn drop(&mut self) {
        IRQ_CHIP.get().unwrap().unmap_irq_line(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct InterruptSourceInFdt {
    pub interrupt_parent: u32,
    pub arguments: [u32; 3],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct InterruptSourceOnChip {
    interrupt_parent: u32,
    interrupt: u32,
}
