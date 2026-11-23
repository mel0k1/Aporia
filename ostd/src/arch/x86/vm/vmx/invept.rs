use x86::{
    msr::{IA32_VMX_EPT_VPID_CAP, IA32_VMX_PROCBASED_CTLS, IA32_VMX_PROCBASED_CTLS2, rdmsr},
    vmx::vmcs::control::{PrimaryControls, SecondaryControls},
};

use super::{VmxGuard, instructions};
use crate::{
    Error,
    cpu::CpuSet,
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, frame::meta::AnyFrameMeta},
    prelude::*,
    smp::{self, PendingIpis},
    sync::{LocalIrqDisabled, RcuDrop, SpinLock},
};

struct PendingInvalidation {
    is_pending: bool,
    frames: Vec<Frame<dyn AnyFrameMeta>>,
}

cpu_local! {
    static PENDING_INVALIDATION: SpinLock<PendingInvalidation, LocalIrqDisabled> =
        SpinLock::new(PendingInvalidation {
            is_pending: false,
            frames: Vec::new(),
        });
}

pub(super) unsafe fn check_ept_support(_irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    let primary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS) } >> 32) as u32;
    if primary_allowed_1 & PrimaryControls::SECONDARY_CONTROLS.bits() == 0 {
        return Err(Error::NotEnoughResources);
    }
    let secondary_allowed_1 = (unsafe { rdmsr(IA32_VMX_PROCBASED_CTLS2) } >> 32) as u32;
    if secondary_allowed_1 & SecondaryControls::ENABLE_EPT.bits() == 0 {
        return Err(Error::NotEnoughResources);
    }

    let cap = unsafe { rdmsr(IA32_VMX_EPT_VPID_CAP) };
    const REQUIRED: u64 = (1 << 6) | (1 << 14) | (1 << 20) | (1 << 26);
    if cap & REQUIRED != REQUIRED {
        return Err(Error::NotEnoughResources);
    }
    Ok(())
}

pub(in crate::arch::vm) fn invalidate(
    _vmx_guard: &VmxGuard,
    frames: &[RcuDrop<Frame<dyn AnyFrameMeta>>],
) -> PendingIpis {
    let targets = CpuSet::new_full();

    for cpu in targets.iter() {
        let mut queue = PENDING_INVALIDATION.get_on_cpu(cpu).lock();
        queue
            .frames
            .extend(frames.iter().map(|frame| (**frame).clone()));
        queue.is_pending = true;
    }

    smp::inter_processor_call(&targets, flush_pending)
}

pub(super) fn flush_pending() {
    let irq_guard = irq::disable_local();
    let frames = {
        let local_queue = PENDING_INVALIDATION.get_with(&irq_guard);
        let mut queue = local_queue.lock();
        if !queue.is_pending {
            return;
        }

        unsafe { instructions::invept_all_contexts(&irq_guard) }.unwrap();
        queue.is_pending = false;
        core::mem::take(&mut queue.frames)
    };
    drop(frames);
}
