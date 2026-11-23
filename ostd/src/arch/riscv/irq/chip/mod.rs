mod plic;

use alloc::boxed::Box;
use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use spin::Once;

use crate::{
    Error, Result,
    arch::{
        boot::DEVICE_TREE,
        irq::{HwIrqLine, InterruptSource, chip::plic::Plic},
    },
    io::IoMemAllocatorBuilder,
    irq::IrqLine,
    sync::{LocalIrqDisabled, SpinLock},
};

pub static IRQ_CHIP: Once<IrqChip> = Once::new();

pub(in crate::arch) unsafe fn init_on_bsp(io_mem_builder: &IoMemAllocatorBuilder) {
    let device_tree = DEVICE_TREE.get().unwrap();
    let mut plics = Plic::from_fdt(device_tree, io_mem_builder);
    plics.iter_mut().for_each(|plic| plic.init());
    IRQ_CHIP.call_once(|| IrqChip {
        plics: SpinLock::new(plics.into_boxed_slice()),
    });
    unsafe { riscv::register::sie::set_sext() };
}

pub(in crate::arch) unsafe fn init_on_ap() {
    unsafe { riscv::register::sie::set_sext() };
}

pub struct IrqChip {
    plics: SpinLock<Box<[Plic]>, LocalIrqDisabled>,
}

impl IrqChip {
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let mut plics = self.plics.lock();
        let (index, plic) = plics
            .iter_mut()
            .enumerate()
            .find(|(_, plic)| plic.phandle() == interrupt_source_in_fdt.interrupt_parent)
            .ok_or(Error::InvalidArgs)?;

        plic.map_interrupt_source_to(interrupt_source_in_fdt.arguments[0], &irq_line)?;
        plic.set_priority(interrupt_source_in_fdt.arguments[0], 1);
        plic.managed_harts().for_each(|hart| {
            plic.set_interrupt_enabled(hart, interrupt_source_in_fdt.arguments[0], true)
        });

        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip: InterruptSourceOnChip {
                index,
                interrupt: interrupt_source_in_fdt.arguments[0],
            },
        })
    }

    pub(in crate::arch) fn claim_interrupt(&self, hart: u32) -> Option<HwIrqLine> {
        self.plics
            .lock()
            .iter()
            .enumerate()
            .find_map(|(index, plic)| {
                let interrupt = plic.claim_interrupt(hart);
                plic.interrupt_number_mapping(interrupt)
                    .map(|irq_num| HwIrqLine {
                        irq_num,
                        source: InterruptSource::External(InterruptSourceOnChip {
                            index,
                            interrupt,
                        }),
                    })
            })
    }

    pub(super) fn complete_interrupt(
        &self,
        hart: u32,
        interrupt_source_on_chip: InterruptSourceOnChip,
    ) {
        let plics = self.plics.lock();
        plics[interrupt_source_on_chip.index]
            .complete_interrupt(hart, interrupt_source_on_chip.interrupt);
    }

    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        let mut plics = self.plics.lock();

        let InterruptSourceOnChip { index, interrupt } = &mapped_irq_line.interrupt_source_on_chip;
        let plic = &mut plics[*index];

        plic.managed_harts()
            .for_each(|hart| plic.set_interrupt_enabled(hart, *interrupt, false));
        plic.set_priority(*interrupt, 0);
        plic.unmap_interrupt_source(*interrupt);
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
    pub arguments: [u32; 1],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct InterruptSourceOnChip {
    index: usize,
    interrupt: u32,
}
