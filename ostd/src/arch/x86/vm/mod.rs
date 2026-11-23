mod context;
mod exit;
pub(crate) mod guest_mode;
pub(crate) mod memory;
mod types;
mod vmcs;
mod vmx;
mod x86;

pub(super) fn init() {
    vmx::init_feature_control();
}

pub use self::{
    context::{GuestContext, VcpuArchState},
    exit::{GuestExitInfo, VmxExitReason},
    types::{
        GuestInterrupt, GuestTimerInstant, VcpuDescTable, VcpuMsrs, VcpuRegs, VcpuSegment,
        VcpuSregs,
    },
};
