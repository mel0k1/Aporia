use spin::Once;

use crate::{cpu::PinCurrentCpu, error::Error, irq::IrqLine, prelude::Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        Self(crate::arch::boot::smp::get_current_hart_id())
    }
}

pub(in crate::arch) static IPI_IRQ: Once<IrqLine> = Once::new();

pub(in crate::arch) unsafe fn init_on_bsp() {
    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(|f| unsafe { crate::smp::do_inter_processor_call(f) });
    IPI_IRQ.call_once(|| irq);

    unsafe { riscv::register::sie::set_ssoft() };
}

pub(in crate::arch) unsafe fn init_on_ap() {
    unsafe { riscv::register::sie::set_ssoft() };
}

pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) -> Result<()> {
    const XLEN: usize = usize::BITS as usize;
    const XLEN_MASK: usize = XLEN - 1;

    let hart_id = hw_cpu_id.0 as usize;
    let hart_mask_base = hart_id & !XLEN_MASK;
    let hart_mask = 1 << (hart_id & XLEN_MASK);

    let ret = sbi_rt::send_ipi(sbi_rt::HartMask::from_mask_base(hart_mask, hart_mask_base));

    if ret.error == 0 {
        crate::debug!("Successfully sent IPI to hart {}", hw_cpu_id.0);
        Ok(())
    } else {
        crate::error!(
            "Failed to send IPI to hart {}: error code {}",
            hw_cpu_id.0,
            ret.error
        );
        Err(Error::IoError)
    }
}
