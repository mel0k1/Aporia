use crate::{cpu::PinCurrentCpu, prelude::Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HwCpuId(u32);

impl HwCpuId {
    pub(crate) fn read_current(_guard: &dyn PinCurrentCpu) -> Self {
        Self(0)
    }
}

pub(crate) fn send_ipi(_hw_cpu_id: HwCpuId, _guard: &dyn PinCurrentCpu) -> Result<()> {
    let _ = crate::smp::do_inter_processor_call;
    unimplemented!()
}
