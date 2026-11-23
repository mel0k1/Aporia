pub(super) mod context_switch;
mod instructions;
pub(super) mod invept;
pub(super) mod vmcs;

use x86::msr::{
    IA32_FEATURE_CONTROL, IA32_VMX_BASIC, IA32_VMX_CR0_FIXED0, IA32_VMX_CR0_FIXED1,
    IA32_VMX_CR4_FIXED0, IA32_VMX_CR4_FIXED1, rdmsr, wrmsr,
};
use x86_64::registers::control::{Cr0, Cr4, Cr4Flags};

use super::vmcs::VMCS_CONTROLS;
use crate::{
    Error,
    arch::cpu::extension::{IsaExtensions, has_extensions},
    cpu::{CpuSet, all_cpus},
    cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::{Frame, FrameAllocOptions, paddr_to_vaddr},
    prelude::*,
    sync::{LocalIrqDisabled, Mutex, SpinLock},
    task::atomic_mode,
};

const FEATURE_CONTROL_LOCKED: u64 = 1;
const FEATURE_CONTROL_VMX_OUTSIDE_SMX: u64 = 1 << 2;

pub(super) fn init_feature_control() {
    if !has_extensions(IsaExtensions::VMX) {
        return;
    }

    let feature_control = unsafe { rdmsr(IA32_FEATURE_CONTROL) };
    if feature_control & FEATURE_CONTROL_LOCKED == 0 {
        unsafe {
            wrmsr(
                IA32_FEATURE_CONTROL,
                FEATURE_CONTROL_LOCKED | FEATURE_CONTROL_VMX_OUTSIDE_SMX,
            );
        }
    }
}

struct VmxonRegionMeta;
crate::impl_frame_meta_for!(VmxonRegionMeta);

struct VmxCpuState {
    is_enabled: bool,
    region: Option<Frame<VmxonRegionMeta>>,
    last_error: Option<Error>,
}

impl VmxCpuState {
    const fn new() -> Self {
        Self {
            is_enabled: false,
            region: None,
            last_error: None,
        }
    }
}

cpu_local! {
    static VMX_CPU_STATE: SpinLock<VmxCpuState, LocalIrqDisabled> =
        SpinLock::new(VmxCpuState::new());
}

struct VmxGuardState {
    active_guards: usize,
    is_poisoned: bool,
}

impl VmxGuardState {
    const fn new() -> Self {
        Self {
            active_guards: 0,
            is_poisoned: false,
        }
    }
}

static VMX_GUARD_STATE: Mutex<VmxGuardState> = Mutex::new(VmxGuardState::new());

#[must_use]
pub(super) struct VmxGuard {
    _private: (),
}

impl VmxGuard {
    pub(super) fn acquire_vmx() -> Result<VmxGuard> {
        atomic_mode::might_sleep();

        let mut state = VMX_GUARD_STATE.lock();
        state.acquire_vmx()?;

        Ok(VmxGuard { _private: () })
    }
}

impl Drop for VmxGuard {
    fn drop(&mut self) {
        atomic_mode::might_sleep();

        let mut state = VMX_GUARD_STATE.lock();
        state.drop_vmx();
    }
}

impl VmxGuardState {
    fn acquire_vmx(&mut self) -> Result<()> {
        if self.is_poisoned {
            return Err(Error::InvalidArgs);
        }
        if self.active_guards == usize::MAX {
            return Err(Error::Overflow);
        }
        if self.active_guards != 0 {
            self.active_guards += 1;
            return Ok(());
        }

        VmxCpuState::prepare_vmxon_regions()?;
        if let Err(err) = self.run_on_all_cpus(VmxCpuState::enable_vmx) {
            let is_rollback_completed = self
                .run_on_all_cpus(|| unsafe { VmxCpuState::disable_vmx() })
                .is_ok();
            VmxCpuState::cleanup_prepared_regions();
            if !is_rollback_completed {
                self.is_poisoned = true;
            }
            return Err(err);
        }

        self.active_guards += 1;

        Ok(())
    }

    fn drop_vmx(&mut self) {
        self.active_guards -= 1;
        if self.active_guards != 0 {
            return;
        }

        if self
            .run_on_all_cpus(|| unsafe { VmxCpuState::disable_vmx() })
            .is_err()
        {
            self.is_poisoned = true;
        }
        VmxCpuState::cleanup_prepared_regions();
    }

    fn run_on_all_cpus(&mut self, handler: fn()) -> Result<()> {
        let targets = CpuSet::new_full();
        crate::smp::inter_processor_call(&targets, handler).wait();
        for cpu in targets.iter() {
            if let Some(error) = VMX_CPU_STATE.get_on_cpu(cpu).lock().last_error {
                return Err(error);
            }
        }
        Ok(())
    }
}

impl VmxCpuState {
    fn enable_vmx() {
        let irq_guard = crate::irq::disable_local();
        let cpu_state = VMX_CPU_STATE.get_with(&irq_guard);
        let mut cpu_state = cpu_state.lock();

        if cpu_state.is_enabled {
            cpu_state.last_error = None;
            return;
        }

        let region_paddr = cpu_state
            .region
            .as_ref()
            .expect("VMX region should be prepared")
            .paddr();

        let cr4 = Cr4::read_raw();
        debug_assert!(cr4 & Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits() == 0);
        let vmx_cr4 = cr4 | Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits();
        let revision_id = match read_and_validate_capability(vmx_cr4, &irq_guard) {
            Ok(revision_id) => revision_id,
            Err(err) => {
                cpu_state.last_error = Some(err);
                return;
            }
        };
        if let Err(err) = unsafe { invept::check_ept_support(&irq_guard) } {
            cpu_state.last_error = Some(err);
            return;
        }

        unsafe { Cr4::write_raw(vmx_cr4) };

        unsafe { initialize_vmxon_region(region_paddr, revision_id) };

        if let Err(error) = unsafe { instructions::vmxon(region_paddr, &irq_guard) } {
            unsafe { clear_vmx_enable(&irq_guard) };
            cpu_state.last_error = Some(error);
            return;
        }

        cpu_state.is_enabled = true;
        cpu_state.last_error = None;
    }

    unsafe fn disable_vmx() {
        let irq_guard = crate::irq::disable_local();
        let cpu_state = VMX_CPU_STATE.get_with(&irq_guard);
        let mut cpu_state = cpu_state.lock();

        if !cpu_state.is_enabled {
            cpu_state.last_error = None;
            return;
        }

        invept::flush_pending();

        if let Err(err) = unsafe { vmcs::LocalVmcsState::deactivate_all(&irq_guard) } {
            cpu_state.last_error = Some(err);
            return;
        }

        if let Err(err) = unsafe { instructions::vmxoff(&irq_guard) } {
            cpu_state.last_error = Some(err);
            return;
        }

        unsafe { clear_vmx_enable(&irq_guard) };

        cpu_state.is_enabled = false;
        cpu_state.last_error = None;
    }

    fn prepare_vmxon_regions() -> Result<()> {
        for cpu in all_cpus() {
            let region = match FrameAllocOptions::new().alloc_frame_with(VmxonRegionMeta) {
                Ok(region) => region,
                Err(error) => {
                    Self::cleanup_prepared_regions();
                    return Err(error);
                }
            };

            let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
            state.set_region(region);
        }

        Ok(())
    }

    fn cleanup_prepared_regions() {
        for cpu in all_cpus() {
            let region = {
                let mut state = VMX_CPU_STATE.get_on_cpu(cpu).lock();
                state.take_region()
            };
            drop(region);
        }
    }

    fn set_region(&mut self, region: Frame<VmxonRegionMeta>) {
        if self.is_enabled {
            return;
        }
        self.region = Some(region);
    }

    fn take_region(&mut self) -> Option<Frame<VmxonRegionMeta>> {
        if self.is_enabled {
            return None;
        }
        self.region.take()
    }
}

fn read_and_validate_capability(vmx_cr4: u64, _irq_guard: &DisabledLocalIrqGuard) -> Result<u32> {
    if !has_extensions(IsaExtensions::VMX) {
        return Err(Error::NotEnoughResources);
    }

    let (feature_control, vmx_basic, cr0_fixed0, cr0_fixed1, cr4_fixed0, cr4_fixed1) = unsafe {
        (
            rdmsr(IA32_FEATURE_CONTROL),
            rdmsr(IA32_VMX_BASIC),
            rdmsr(IA32_VMX_CR0_FIXED0),
            rdmsr(IA32_VMX_CR0_FIXED1),
            rdmsr(IA32_VMX_CR4_FIXED0),
            rdmsr(IA32_VMX_CR4_FIXED1),
        )
    };

    if feature_control & FEATURE_CONTROL_VMX_OUTSIDE_SMX == 0 {
        return Err(Error::AccessDenied);
    }

    if !control_register_is_valid(Cr0::read_raw(), cr0_fixed0, cr0_fixed1)
        || !control_register_is_valid(vmx_cr4, cr4_fixed0, cr4_fixed1)
    {
        return Err(Error::NotEnoughResources);
    }

    const VMCS_MEMORY_TYPE_WB: u64 = 6;
    if (vmx_basic >> 50) & 0xf != VMCS_MEMORY_TYPE_WB {
        return Err(Error::NotEnoughResources);
    }

    for control in &VMCS_CONTROLS {
        let capability = unsafe { rdmsr(control.capability_msr(vmx_basic)) };
        let can_set = control.required_bits | control.runtime_bits;
        let can_clear = control.forbidden_bits | control.runtime_bits;
        if (capability >> 32) as u32 & can_set != can_set || capability as u32 & can_clear != 0 {
            return Err(Error::NotEnoughResources);
        }
    }

    Ok(vmx_basic as u32 & 0x7fff_ffff)
}

fn control_register_is_valid(value: u64, fixed0: u64, fixed1: u64) -> bool {
    value & fixed0 == fixed0 && value & !fixed1 == 0
}

unsafe fn initialize_vmxon_region(region_paddr: usize, revision_id: u32) {
    let region_ptr = paddr_to_vaddr(region_paddr) as *mut u32;

    unsafe { region_ptr.write(revision_id) };
}

unsafe fn clear_vmx_enable(_irq_guard: &DisabledLocalIrqGuard) {
    let cr4 = Cr4::read_raw();
    unsafe { Cr4::write_raw(cr4 & !Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits()) };
}

#[cfg(ktest)]
mod test {
    use super::*;

    #[ktest]
    fn multiple_vmx_guards_lifecycle() {
        if !has_extensions(IsaExtensions::VMX) {
            crate::early_print!(" [skipped: VMX unavailable]");
            return;
        }

        let guard1 = VmxGuard::acquire_vmx().expect("Failed to acquire VMX guard");
        assert_eq!(
            unsafe { instructions::vmptrst(&crate::irq::disable_local()) },
            u64::MAX
        );

        let guard2 = VmxGuard::acquire_vmx().expect("Failed to acquire VMX guard");
        assert_eq!(
            unsafe { instructions::vmptrst(&crate::irq::disable_local()) },
            u64::MAX
        );

        drop(guard1);
        assert_eq!(
            unsafe { instructions::vmptrst(&crate::irq::disable_local()) },
            u64::MAX
        );

        drop(guard2);
        let irq_guard = crate::irq::disable_local();
        let state = VMX_CPU_STATE.get_with(&irq_guard);
        assert!(!state.lock().is_enabled);
        let cr4 = Cr4::read_raw();
        assert_eq!(cr4 & Cr4Flags::VIRTUAL_MACHINE_EXTENSIONS.bits(), 0);
    }
}
