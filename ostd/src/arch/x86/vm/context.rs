use x86::msr::{IA32_FMASK, IA32_LSTAR, IA32_STAR, wrmsr};
use x86_64::{VirtAddr, registers::control::Cr2};

use super::{
    GuestInterrupt, GuestTimerInstant, VcpuMsrs, VcpuRegs, VcpuSregs,
    guest_mode::SyscallMsrsRestorer,
    vmx::{VmxGuard, vmcs::Vmcs},
    x86::write_cr2_raw,
};
use crate::{Error, arch::cpu::context::GsBase, irq::DisabledLocalIrqGuard, prelude::*};

pub struct GuestContext {
    arch: VcpuArchState,
    vmcs: Vmcs,
}

impl GuestContext {
    pub fn new(regs: VcpuRegs, sregs: VcpuSregs, msrs: VcpuMsrs) -> Result<Self> {
        let vmx_guard = VmxGuard::acquire_vmx()?;
        Ok(Self {
            arch: VcpuArchState {
                regs,
                sregs,
                msrs,
                pending_interrupt: None,
                timer_deadline: None,
            },
            vmcs: Vmcs::new(&vmx_guard)?,
        })
    }

    pub fn regs(&self) -> &VcpuRegs {
        &self.arch.regs
    }

    pub fn regs_mut(&mut self) -> &mut VcpuRegs {
        &mut self.arch.regs
    }

    pub fn sregs(&self) -> &VcpuSregs {
        &self.arch.sregs
    }

    pub fn sregs_mut(&mut self) -> &mut VcpuSregs {
        &mut self.arch.sregs
    }

    pub fn msrs(&self) -> &VcpuMsrs {
        &self.arch.msrs
    }

    pub fn msrs_mut(&mut self) -> &mut VcpuMsrs {
        &mut self.arch.msrs
    }

    pub(super) fn arch_and_vmcs(&mut self) -> (&mut VcpuArchState, &Vmcs) {
        (&mut self.arch, &self.vmcs)
    }
}

impl Drop for GuestContext {
    fn drop(&mut self) {
        if let Err(err) =
            VmxGuard::acquire_vmx().and_then(|vmx_guard| self.vmcs.deactivate(&vmx_guard))
        {
            error!("failed to clear VMCS during context destruction: {:?}", err);
        }
    }
}

pub struct VcpuArchState {
    pub regs: VcpuRegs,
    pub sregs: VcpuSregs,
    pub msrs: VcpuMsrs,
    pub pending_interrupt: Option<GuestInterrupt>,
    pub timer_deadline: Option<GuestTimerInstant>,
}

impl VcpuArchState {
    pub(super) unsafe fn load_run_state<'a>(
        &self,
        irq_guard: &'a DisabledLocalIrqGuard,
    ) -> Result<SyscallMsrsRestorer<'a>> {
        for value in [self.msrs.kernel_gs_base, self.msrs.lstar] {
            if VirtAddr::try_new(value).is_err() {
                return Err(Error::InvalidArgs);
            }
        }
        if self.msrs.syscall_mask > u32::MAX as u64 {
            return Err(Error::InvalidArgs);
        }

        let syscall_msrs_restorer = SyscallMsrsRestorer::new(irq_guard);
        unsafe {
            wrmsr(IA32_STAR, self.msrs.star);
            wrmsr(IA32_LSTAR, self.msrs.lstar);
            wrmsr(IA32_FMASK, self.msrs.syscall_mask);
        }

        write_cr2_raw(self.sregs.cr2);
        GsBase::new(self.msrs.kernel_gs_base as usize).load(irq_guard);
        Ok(syscall_msrs_restorer)
    }

    pub(super) fn save_run_state(&mut self, irq_guard: &DisabledLocalIrqGuard) {
        self.sregs.cr2 = Cr2::read_raw();
        self.msrs.kernel_gs_base = {
            let mut kernel_gs_base = GsBase::default();
            kernel_gs_base.save(irq_guard);
            kernel_gs_base.addr() as u64
        };
    }
}
