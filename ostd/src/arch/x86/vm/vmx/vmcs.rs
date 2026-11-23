use alloc::collections::BTreeMap;
use core::{
    cell::{RefCell, RefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use x86::{
    msr::{IA32_VMX_BASIC, rdmsr},
    vmx::vmcs::ro,
};

use super::{VmxGuard, context_switch, instructions};
use crate::{
    Error,
    cpu::{CpuId, CpuSet, PinCurrentCpu},
    cpu_local,
    irq::{self, DisabledLocalIrqGuard},
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, PreemptDisabled, SpinLock, SpinLockGuard},
    task::{self, atomic_mode::AsAtomicModeGuard},
};

pub(in crate::arch::vm) type Vmcs = Frame<VmcsRegionMeta>;

pub(in crate::arch::vm) struct VmcsRegionMeta {
    state: SpinLock<VmcsState>,
    is_launched: AtomicBool,
}
crate::impl_frame_meta_for!(VmcsRegionMeta);

struct VmcsState {
    active_cpu: Option<CpuId>,
    are_fixed_fields_initialized: bool,
}

impl Vmcs {
    pub(in crate::arch::vm) fn new(_vmx_guard: &VmxGuard) -> Result<Self> {
        let revision_id = unsafe { rdmsr(IA32_VMX_BASIC) } as u32 & 0x7fff_ffff;
        let frame = FrameAllocOptions::new().alloc_frame_with(VmcsRegionMeta {
            state: SpinLock::new(VmcsState {
                active_cpu: None,
                are_fixed_fields_initialized: false,
            }),
            is_launched: AtomicBool::new(false),
        })?;
        unsafe { (paddr_to_vaddr(frame.paddr()) as *mut u32).write(revision_id) };

        Ok(frame)
    }

    pub(in crate::arch::vm) fn load<'a>(
        &'a self,
        vmx_guard: &'a VmxGuard,
        irq_guard: &'a mut Option<DisabledLocalIrqGuard>,
    ) -> Result<CurrentVmcs<'a>> {
        assert!(crate::arch::irq::is_local_enabled());
        debug_assert!(irq_guard.is_none());

        let mut state = self.meta().state.lock();

        if state.active_cpu != Some(state.as_atomic_mode_guard().current_cpu()) {
            state.are_fixed_fields_initialized = false;
        }

        let current =
            unsafe { LocalVmcsState::do_activate(self, &mut state, vmx_guard, irq_guard) }?;

        if !state.are_fixed_fields_initialized {
            current.setup_fixed_controls(vmx_guard)?;
            current.setup_fixed_host()?;
            state.are_fixed_fields_initialized = true;
        }

        Ok(current)
    }

    pub(in crate::arch::vm) fn deactivate(&self, vmx_guard: &VmxGuard) -> Result<()> {
        assert!(crate::arch::irq::is_local_enabled());
        unsafe { LocalVmcsState::do_deactivate(self, &mut self.meta().state.lock(), vmx_guard) }
    }
}

pub(in crate::arch::vm) struct CurrentVmcs<'a> {
    vmcs: &'a Vmcs,
    irq_guard: &'a DisabledLocalIrqGuard,
    _local_guard: RefMut<'a, LocalVmcsState>,
    _vmx_guard: &'a VmxGuard,
}

impl CurrentVmcs<'_> {
    pub(in crate::arch::vm) fn irq_guard(&self) -> &DisabledLocalIrqGuard {
        self.irq_guard
    }

    pub(in crate::arch::vm) fn read(&self, field: u32) -> Result<usize> {
        unsafe { instructions::vmread(field, self.irq_guard) }
    }

    pub(in crate::arch::vm) unsafe fn write(&self, field: u32, value: usize) -> Result<()> {
        unsafe { instructions::vmwrite(field, value, self.irq_guard) }
    }

    pub(in crate::arch::vm) unsafe fn run(
        &self,
        regs: &mut crate::arch::vm::VcpuRegs,
    ) -> Result<()> {
        let is_launched = self.vmcs.meta().is_launched.load(Ordering::Relaxed);
        let result = unsafe { context_switch::vcpu_run(regs, is_launched, self.irq_guard) };
        if result != 0 {
            return Err(Error::InvalidArgs);
        }
        if self.read(ro::EXIT_REASON)? & (1 << 31) == 0 {
            self.vmcs.meta().is_launched.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

pub(super) struct LocalVmcsState {
    current: Option<Paddr>,
    active: BTreeMap<Paddr, Vmcs>,
}

cpu_local! {
    static LOCAL_VMCS_STATE: RefCell<LocalVmcsState> =
        RefCell::new(LocalVmcsState::new());
    static PENDING_CLEAR: SpinLock<Vec<Paddr>, LocalIrqDisabled> =
        SpinLock::new(Vec::new());
}

impl LocalVmcsState {
    const fn new() -> Self {
        Self {
            current: None,
            active: BTreeMap::new(),
        }
    }

    unsafe fn do_activate<'a>(
        vmcs: &'a Vmcs,
        vmcs_state: &mut SpinLockGuard<'_, VmcsState, PreemptDisabled>,
        vmx_guard: &'a VmxGuard,
        irq_guard: &'a mut Option<DisabledLocalIrqGuard>,
    ) -> Result<CurrentVmcs<'a>> {
        let previous_cpu = vmcs_state.active_cpu;
        if let Some(active_cpu) = previous_cpu
            && active_cpu != vmcs_state.as_atomic_mode_guard().current_cpu()
        {
            unsafe { Self::do_deactivate(vmcs, vmcs_state, vmx_guard)? };
        }

        let irq_guard = irq_guard.insert(irq::disable_local());
        let mut local = LOCAL_VMCS_STATE
            .get_with(irq_guard)
            .deref_target()
            .borrow_mut();
        if local.current == Some(vmcs.paddr()) {
            return Ok(CurrentVmcs {
                vmcs,
                irq_guard,
                _local_guard: local,
                _vmx_guard: vmx_guard,
            });
        }

        let was_active = local.active.contains_key(&vmcs.paddr());
        if !was_active {
            if previous_cpu.is_none() {
                unsafe { instructions::vmclear(vmcs.paddr(), irq_guard) }?;
                vmcs.meta().is_launched.store(false, Ordering::Relaxed);
            }
            local.active.insert(vmcs.paddr(), vmcs.clone());
        }

        if let Err(err) = unsafe { instructions::vmptrld(vmcs.paddr(), irq_guard) } {
            if !was_active {
                local.active.remove(&vmcs.paddr());
            }
            return Err(err);
        }
        local.current = Some(vmcs.paddr());
        vmcs_state.active_cpu = Some(vmcs_state.as_atomic_mode_guard().current_cpu());
        Ok(CurrentVmcs {
            vmcs,
            irq_guard,
            _local_guard: local,
            _vmx_guard: vmx_guard,
        })
    }

    unsafe fn do_deactivate(
        vmcs: &Vmcs,
        vmcs_state: &mut SpinLockGuard<'_, VmcsState, PreemptDisabled>,
        _vmx_guard: &VmxGuard,
    ) -> Result<()> {
        let Some(cpu) = vmcs_state.active_cpu else {
            return Ok(());
        };
        let preempt_guard = task::disable_preempt();
        if cpu == preempt_guard.current_cpu() {
            let irq_guard = irq::disable_local();
            unsafe {
                LOCAL_VMCS_STATE
                    .get_with(&irq_guard)
                    .borrow_mut()
                    .clear(vmcs.paddr(), &irq_guard)?
            };
            vmcs_state.active_cpu = None;
            vmcs.meta().is_launched.store(false, Ordering::Relaxed);
            return Ok(());
        }

        assert!(crate::arch::irq::is_local_enabled());
        PENDING_CLEAR.get_on_cpu(cpu).lock().push(vmcs.paddr());

        crate::smp::inter_processor_call(&CpuSet::from(cpu), || {
            let irq_guard = irq::disable_local();
            let pending_clear = PENDING_CLEAR.get_with(&irq_guard);
            let mut pending_clear = pending_clear.lock();
            let local = LOCAL_VMCS_STATE.get_with(&irq_guard);
            let mut local = local.borrow_mut();
            while let Some(paddr) = pending_clear.pop() {
                if let Err(err) = unsafe { local.clear(paddr, &irq_guard) } {
                    panic!("failed to clear remote VMCS: {:?}", err);
                }
            }
        })
        .wait();
        vmcs_state.active_cpu = None;
        vmcs.meta().is_launched.store(false, Ordering::Relaxed);
        Ok(())
    }

    unsafe fn clear(&mut self, paddr: Paddr, irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        unsafe { instructions::vmclear(paddr, irq_guard) }?;

        self.active.remove(&paddr);
        if self.current == Some(paddr) {
            self.current = None;
        }
        Ok(())
    }

    pub(super) unsafe fn deactivate_all(irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        let local = LOCAL_VMCS_STATE.get_with(irq_guard);
        let active = {
            let mut local = local.borrow_mut();
            local.current = None;
            core::mem::take(&mut local.active)
        };
        for (paddr, vmcs) in active {
            unsafe { instructions::vmclear(paddr, irq_guard) }?;
            let mut state = vmcs.meta().state.lock();
            state.active_cpu = None;
            vmcs.meta().is_launched.store(false, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[cfg(ktest)]
mod test {
    use x86::vmx::vmcs::guest::RIP;

    use super::*;
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

    #[ktest]
    fn switch_and_reuse_vmcs() {
        if !has_extensions(IsaExtensions::VMX) {
            crate::early_print!(" [skipped: VMX unavailable]");
            return;
        }

        let vmx = VmxGuard::acquire_vmx().expect("VMX is required");
        let first = Vmcs::new(&vmx).unwrap();
        let second = Vmcs::new(&vmx).unwrap();

        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            unsafe { current.write(RIP, 0x1000).unwrap() };
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            unsafe { current.write(RIP, 0x2000).unwrap() };
        }
        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x1000);
        }

        drop(vmx);
        assert!(first.meta().state.lock().active_cpu.is_none());
        assert!(second.meta().state.lock().active_cpu.is_none());
        let vmx = VmxGuard::acquire_vmx().unwrap();
        let preempt_guard = task::disable_preempt();
        {
            let mut irq_guard = None;
            let current = first.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x1000);
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x2000);
        }

        first.deactivate(&vmx).unwrap();
        drop(first);
        {
            let irq_guard = irq::disable_local();
            assert_eq!(
                unsafe { instructions::vmptrst(&irq_guard) },
                second.paddr() as u64
            );
        }
        {
            let mut irq_guard = None;
            let current = second.load(&vmx, &mut irq_guard).unwrap();
            assert_eq!(current.read(RIP).unwrap(), 0x2000);
        }
        second.deactivate(&vmx).unwrap();
        drop(second);
        drop(preempt_guard);
    }
}
