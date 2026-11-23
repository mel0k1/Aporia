use super::GuestPhysMemSpace;
use crate::{
    arch::vm::{GuestContext, GuestExitInfo, VcpuArchState, guest_mode::ArchGuestMode},
    irq::DisabledLocalIrqGuard,
    prelude::*,
    task,
};

pub struct GuestMode {
    arch: ArchGuestMode,
}

impl GuestMode {
    pub fn new() -> Result<Self> {
        Ok(Self {
            arch: ArchGuestMode::new()?,
        })
    }

    #[track_caller]
    pub fn execute<H: GuestModeHooks + ?Sized>(
        &self,
        context: &mut GuestContext,
        guest_mem: &GuestPhysMemSpace,
        hooks: &H,
    ) -> Result<GuestReturnReason> {
        task::atomic_mode::might_sleep();

        loop {
            task::scheduler::might_preempt();

            if let Some(exit) = self.arch.execute_once(context, guest_mem, hooks)? {
                return Ok(GuestReturnReason::VmExit(exit));
            }

            if hooks.has_kernel_event() {
                return Ok(GuestReturnReason::KernelEvent);
            }
        }
    }
}

pub trait GuestModeHooks {
    fn pre_guest_run(&self, state: &mut VcpuArchState, guard: &DisabledLocalIrqGuard);

    fn post_guest_run(&self, state: &mut VcpuArchState, guard: &DisabledLocalIrqGuard);

    fn has_kernel_event(&self) -> bool;
}

pub struct DummyGuestHooks;

impl GuestModeHooks for DummyGuestHooks {
    fn pre_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn post_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn has_kernel_event(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub enum GuestReturnReason {
    VmExit(GuestExitInfo),
    KernelEvent,
}
