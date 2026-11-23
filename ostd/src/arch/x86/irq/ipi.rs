use spin::Once;

use crate::{cpu::PinCurrentCpu, irq::IrqLine, prelude::Result, smp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(guard: &dyn PinCurrentCpu) -> Self {
        use crate::arch::kernel::apic;

        let apic = apic::get_or_init(guard);
        Self(apic.id())
    }
}

static IPI_IRQ: Once<IrqLine> = Once::new();

pub(in crate::arch) fn init() {
    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(|f| unsafe { smp::do_inter_processor_call(f) });
    IPI_IRQ.call_once(|| irq);
}

pub(crate) fn send_ipi(hw_cpu_id: HwCpuId, guard: &dyn PinCurrentCpu) -> Result<()> {
    use crate::arch::kernel::apic::{self, Icr};

    let irq_num = IPI_IRQ.get().unwrap().num();

    let icr = Icr::new(
        apic::ApicId::from(hw_cpu_id.0),
        apic::DestinationShorthand::NoShorthand,
        apic::TriggerMode::Edge,
        apic::Level::Assert,
        apic::DeliveryStatus::Idle,
        apic::DestinationMode::Physical,
        apic::DeliveryMode::Fixed,
        irq_num,
    );

    let apic = apic::get_or_init(guard);
    unsafe { apic.send_ipi(icr) };

    Ok(())
}
