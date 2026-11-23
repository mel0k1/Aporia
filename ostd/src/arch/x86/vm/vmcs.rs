use ::x86::{
    msr, segmentation,
    vmx::vmcs::{
        control::{
            self, EntryControls, ExitControls, PinbasedControls, PrimaryControls, SecondaryControls,
        },
        guest, host,
    },
};
use x86_64::registers::{
    control::{Cr0, Cr0Flags, Cr4, Cr4Flags},
    model_specific::EferFlags,
};

use super::{
    context::VcpuArchState,
    types::VcpuSegment,
    vmx::{VmxGuard, context_switch, vmcs::CurrentVmcs},
    x86,
};
use crate::{
    Error,
    arch::trap::{gdt, idt},
    irq::DisabledLocalIrqGuard,
    prelude::*,
};

pub(super) struct VmcsControl {
    field: u32,
    msr: u32,
    msr_with_true_controls: u32,
    pub(super) required_bits: u32,
    pub(super) forbidden_bits: u32,
    pub(super) runtime_bits: u32,
}

impl VmcsControl {
    pub(super) fn capability_msr(&self, vmx_basic: u64) -> u32 {
        if vmx_basic & (1 << 55) != 0 {
            self.msr_with_true_controls
        } else {
            self.msr
        }
    }
}

pub(super) const VMCS_CONTROLS: [VmcsControl; 5] = [
    VmcsControl {
        field: control::PINBASED_EXEC_CONTROLS,
        msr: msr::IA32_VMX_PINBASED_CTLS,
        msr_with_true_controls: msr::IA32_VMX_TRUE_PINBASED_CTLS,
        required_bits: PinbasedControls::EXTERNAL_INTERRUPT_EXITING.bits()
            | PinbasedControls::NMI_EXITING.bits()
            | PinbasedControls::VMX_PREEMPTION_TIMER.bits(),
        forbidden_bits: 0,
        runtime_bits: 0,
    },
    VmcsControl {
        field: control::PRIMARY_PROCBASED_EXEC_CONTROLS,
        msr: msr::IA32_VMX_PROCBASED_CTLS,
        msr_with_true_controls: msr::IA32_VMX_TRUE_PROCBASED_CTLS,
        required_bits: PrimaryControls::HLT_EXITING.bits()
            | PrimaryControls::CR8_LOAD_EXITING.bits()
            | PrimaryControls::CR8_STORE_EXITING.bits()
            | PrimaryControls::MOV_DR_EXITING.bits()
            | PrimaryControls::UNCOND_IO_EXITING.bits()
            | PrimaryControls::SECONDARY_CONTROLS.bits(),
        forbidden_bits: PrimaryControls::CR3_LOAD_EXITING.bits()
            | PrimaryControls::CR3_STORE_EXITING.bits(),
        runtime_bits: 0,
    },
    VmcsControl {
        field: control::SECONDARY_PROCBASED_EXEC_CONTROLS,
        msr: msr::IA32_VMX_PROCBASED_CTLS2,
        msr_with_true_controls: msr::IA32_VMX_PROCBASED_CTLS2,
        required_bits: SecondaryControls::ENABLE_EPT.bits()
            | SecondaryControls::UNRESTRICTED_GUEST.bits(),
        forbidden_bits: 0,
        runtime_bits: 0,
    },
    VmcsControl {
        field: control::VMEXIT_CONTROLS,
        msr: msr::IA32_VMX_EXIT_CTLS,
        msr_with_true_controls: msr::IA32_VMX_TRUE_EXIT_CTLS,
        required_bits: ExitControls::HOST_ADDRESS_SPACE_SIZE.bits()
            | ExitControls::SAVE_IA32_PAT.bits()
            | ExitControls::LOAD_IA32_PAT.bits()
            | ExitControls::SAVE_IA32_EFER.bits()
            | ExitControls::LOAD_IA32_EFER.bits(),
        forbidden_bits: 0,
        runtime_bits: 0,
    },
    VmcsControl {
        field: control::VMENTRY_CONTROLS,
        msr: msr::IA32_VMX_ENTRY_CTLS,
        msr_with_true_controls: msr::IA32_VMX_TRUE_ENTRY_CTLS,
        required_bits: EntryControls::LOAD_IA32_PAT.bits() | EntryControls::LOAD_IA32_EFER.bits(),
        forbidden_bits: 0,
        runtime_bits: EntryControls::IA32E_MODE_GUEST.bits(),
    },
];

macro_rules! write_vmcs_fields {
    ($vmcs:ident; $( [$field:expr, $value:expr] ),+ $(,)?) => {{
        $( $vmcs.write($field, $value as usize)?; )+
        Result::<()>::Ok(())
    }};
}

macro_rules! read_vmcs_fields {
    ($vmcs:ident; $( [$field:expr, $dest:expr] ),+ $(,)?) => {
        $( $dest = $vmcs.read($field)? as _; )+
    };
}

impl CurrentVmcs<'_> {
    pub(super) fn setup_fixed_controls(&self, _vmx_guard: &VmxGuard) -> Result<()> {
        let vmx_basic = unsafe { msr::rdmsr(msr::IA32_VMX_BASIC) };

        for control in &VMCS_CONTROLS {
            let capability = unsafe { msr::rdmsr(control.capability_msr(vmx_basic)) };
            let value = (control.required_bits | capability as u32) & (capability >> 32) as u32;

            unsafe { self.write(control.field, value as usize) }?;
        }

        unsafe {
            write_vmcs_fields!(self;
                [control::EXCEPTION_BITMAP, 0],
                [control::PAGE_FAULT_ERR_CODE_MASK, 0],
                [control::PAGE_FAULT_ERR_CODE_MATCH, 0],
                [control::CR3_TARGET_COUNT, 0],

                [control::VMEXIT_MSR_STORE_COUNT, 0],
                [control::VMEXIT_MSR_LOAD_COUNT, 0],
                [control::VMENTRY_MSR_LOAD_COUNT, 0],
            )
        }
    }

    pub(super) unsafe fn sync_controls(&self, eptp: u64, guest_efer: u64) -> Result<()> {
        const EPT_MEM_TYPE_WB: u64 = 6;
        const EPT_PAGE_WALK_LENGTH_4_LEVELS: u64 = 3 << 3;

        let mut entry = self.read(control::VMENTRY_CONTROLS)?
            & !(EntryControls::IA32E_MODE_GUEST.bits() as usize);
        if guest_efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0 {
            entry |= EntryControls::IA32E_MODE_GUEST.bits() as usize;
        }

        let eptp = eptp | EPT_MEM_TYPE_WB | EPT_PAGE_WALK_LENGTH_4_LEVELS;

        unsafe {
            write_vmcs_fields!(self;
                [control::VMENTRY_CONTROLS, entry],
                [control::EPTP_FULL, eptp],
            )
        }
    }

    pub(super) fn setup_fixed_host(&self) -> Result<()> {
        use x86_64::registers::segmentation::{GS, Segment64};

        unsafe {
            write_vmcs_fields!(self;
                [host::CS_SELECTOR, gdt::KERNEL_CS.0],
                [host::GS_BASE, GS::read_base().as_u64()],

                [host::IA32_PAT_FULL, msr::rdmsr(msr::IA32_PAT)],
                [host::IA32_EFER_FULL, msr::rdmsr(msr::IA32_EFER)],
                [host::IA32_SYSENTER_CS, 0],
                [host::IA32_SYSENTER_ESP, 0],
                [host::IA32_SYSENTER_EIP, 0],

                [host::TR_SELECTOR, gdt::TSS_SEL.0],
                [host::TR_BASE, gdt::tss_base(self.irq_guard())],
                [host::GDTR_BASE, gdt::gdt_base(self.irq_guard())],
                [host::IDTR_BASE, idt::idt_base()],

                [host::RIP, context_switch::vm_exit_handler_virtaddr()],
            )
        }
    }

    pub(super) fn sync_host(&self) -> Result<()> {
        use x86_64::registers::segmentation::{FS, Segment64};

        unsafe {
            write_vmcs_fields!(self;
                [host::CR0, Cr0::read_raw()],
                [host::CR3, x86::read_cr3_raw()],
                [host::CR4, Cr4::read_raw()],

                [host::ES_SELECTOR, segmentation::es().bits()],
                [host::SS_SELECTOR, segmentation::ss().bits()],
                [host::DS_SELECTOR, segmentation::ds().bits()],
                [host::FS_SELECTOR, segmentation::fs().bits()],
                [host::GS_SELECTOR, segmentation::gs().bits()],
                [host::FS_BASE, FS::read_base().as_u64()],
            )
        }
    }

    pub(super) fn load_guest_context(
        &self,
        arch: &VcpuArchState,
        vmx_guard: &VmxGuard,
    ) -> Result<()> {
        let sregs = &arch.sregs;

        if sregs.cr0 & Cr0Flags::PAGING.bits() != 0
            && sregs.cr4 & Cr4Flags::PHYSICAL_ADDRESS_EXTENSION.bits() != 0
            && sregs.efer & EferFlags::LONG_MODE_ACTIVE.bits() == 0
        {
            return Err(Error::InvalidArgs);
        }

        unsafe {
            self.write_segment(&sregs.cs, CS)?;
            self.write_segment(&sregs.ds, DS)?;
            self.write_segment(&sregs.es, ES)?;
            self.write_segment(&sregs.fs, FS)?;
            self.write_segment(&sregs.gs, GS)?;
            self.write_segment(&sregs.ss, SS)?;
            self.write_segment(&sregs.tr, TR)?;
            self.write_segment(&sregs.ldt, LDTR)?;

            write_vmcs_fields!(self;
                [guest::CR0, fix_guest_cr0(sregs.cr0, vmx_guard, self.irq_guard())],
                [guest::CR4, fix_guest_cr4(sregs.cr4, vmx_guard, self.irq_guard())],
                [control::CR0_GUEST_HOST_MASK, usize::MAX],
                [control::CR4_GUEST_HOST_MASK, usize::MAX],
                [control::CR0_READ_SHADOW, sregs.cr0],
                [control::CR4_READ_SHADOW, sregs.cr4],
                [guest::CR3, sregs.cr3],

                [guest::GDTR_BASE, sregs.gdt.base],
                [guest::GDTR_LIMIT, sregs.gdt.limit],
                [guest::IDTR_BASE, sregs.idt.base],
                [guest::IDTR_LIMIT, sregs.idt.limit],

                [guest::RIP, arch.regs.rip],
                [guest::RSP, arch.regs.rsp],
                [guest::RFLAGS, arch.regs.rflags | 2],

                [guest::IA32_EFER_FULL, sregs.efer],
                [guest::IA32_PAT_FULL, arch.msrs.pat],
                [guest::IA32_SYSENTER_CS, arch.msrs.sysenter_cs],
                [guest::IA32_SYSENTER_ESP, arch.msrs.sysenter_esp],
                [guest::IA32_SYSENTER_EIP, arch.msrs.sysenter_eip],

                [guest::ACTIVITY_STATE, 0],
                [guest::LINK_PTR_FULL, usize::MAX],
            )
        }
    }

    pub(super) fn save_guest_context(&self, arch: &mut VcpuArchState) -> Result<()> {
        let (sregs, regs) = (&mut arch.sregs, &mut arch.regs);

        sregs.cs = self.read_segment(CS)?;
        sregs.ds = self.read_segment(DS)?;
        sregs.es = self.read_segment(ES)?;
        sregs.fs = self.read_segment(FS)?;
        sregs.gs = self.read_segment(GS)?;
        sregs.ss = self.read_segment(SS)?;
        sregs.tr = self.read_segment(TR)?;
        sregs.ldt = self.read_segment(LDTR)?;

        read_vmcs_fields!(self;
            [guest::GDTR_BASE, sregs.gdt.base],
            [guest::GDTR_LIMIT, sregs.gdt.limit],
            [guest::IDTR_BASE, sregs.idt.base],
            [guest::IDTR_LIMIT, sregs.idt.limit],
            [guest::CR3, sregs.cr3],

            [guest::RIP, regs.rip],
            [guest::RSP, regs.rsp],
            [guest::RFLAGS, regs.rflags],
        );

        Ok(())
    }

    unsafe fn write_segment(
        &self,
        segment: &VcpuSegment,
        [selector, base, limit, rights]: [u32; 4],
    ) -> Result<()> {
        unsafe {
            write_vmcs_fields!(self;
                [selector, segment.selector],
                [base, segment.base],
                [limit, segment.limit],
                [rights, segment.vmcs_access_rights()],
            )
        }
    }

    fn read_segment(&self, [selector, base, limit, rights]: [u32; 4]) -> Result<VcpuSegment> {
        let mut segment = VcpuSegment::default();
        let access: usize;

        read_vmcs_fields!(self;
            [rights, access],
            [base, segment.base],
            [limit, segment.limit],
            [selector, segment.selector],
        );

        segment.set_vmcs_access_rights(access);
        Ok(segment)
    }
}

impl VcpuSegment {
    fn vmcs_access_rights(&self) -> usize {
        let mut access = usize::from(self.type_ & 0xf);
        access |= usize::from(self.s & 1) << 4;
        access |= usize::from(self.dpl & 3) << 5;
        access |= usize::from(self.present & 1) << 7;
        access |= usize::from(self.avl & 1) << 12;
        access |= usize::from(self.l & 1) << 13;
        access |= usize::from(self.db & 1) << 14;
        access |= usize::from(self.g & 1) << 15;
        access |= usize::from(self.unusable & 1) << 16;
        access
    }

    fn set_vmcs_access_rights(&mut self, access: usize) {
        self.type_ = (access & 0xf) as u8;
        self.present = ((access >> 7) & 1) as u8;
        self.dpl = ((access >> 5) & 3) as u8;
        self.db = ((access >> 14) & 1) as u8;
        self.s = ((access >> 4) & 1) as u8;
        self.l = ((access >> 13) & 1) as u8;
        self.g = ((access >> 15) & 1) as u8;
        self.avl = ((access >> 12) & 1) as u8;
        self.unusable = ((access >> 16) & 1) as u8;
    }
}

fn fix_guest_cr0(cr0: u64, _vmx_guard: &VmxGuard, _irq_guard: &DisabledLocalIrqGuard) -> u64 {
    let (fixed0, fixed1) = unsafe {
        (
            msr::rdmsr(msr::IA32_VMX_CR0_FIXED0),
            msr::rdmsr(msr::IA32_VMX_CR0_FIXED1),
        )
    };
    let fixed0 = fixed0 & !(Cr0Flags::PROTECTED_MODE_ENABLE | Cr0Flags::PAGING).bits();
    (cr0 | fixed0) & fixed1
}

fn fix_guest_cr4(cr4: u64, _vmx_guard: &VmxGuard, _irq_guard: &DisabledLocalIrqGuard) -> u64 {
    let (fixed0, fixed1) = unsafe {
        (
            msr::rdmsr(msr::IA32_VMX_CR4_FIXED0),
            msr::rdmsr(msr::IA32_VMX_CR4_FIXED1),
        )
    };
    (cr4 | fixed0) & fixed1
}

const CS: [u32; 4] = [
    guest::CS_SELECTOR,
    guest::CS_BASE,
    guest::CS_LIMIT,
    guest::CS_ACCESS_RIGHTS,
];
const DS: [u32; 4] = [
    guest::DS_SELECTOR,
    guest::DS_BASE,
    guest::DS_LIMIT,
    guest::DS_ACCESS_RIGHTS,
];
const ES: [u32; 4] = [
    guest::ES_SELECTOR,
    guest::ES_BASE,
    guest::ES_LIMIT,
    guest::ES_ACCESS_RIGHTS,
];
const FS: [u32; 4] = [
    guest::FS_SELECTOR,
    guest::FS_BASE,
    guest::FS_LIMIT,
    guest::FS_ACCESS_RIGHTS,
];
const GS: [u32; 4] = [
    guest::GS_SELECTOR,
    guest::GS_BASE,
    guest::GS_LIMIT,
    guest::GS_ACCESS_RIGHTS,
];
const SS: [u32; 4] = [
    guest::SS_SELECTOR,
    guest::SS_BASE,
    guest::SS_LIMIT,
    guest::SS_ACCESS_RIGHTS,
];
const TR: [u32; 4] = [
    guest::TR_SELECTOR,
    guest::TR_BASE,
    guest::TR_LIMIT,
    guest::TR_ACCESS_RIGHTS,
];
const LDTR: [u32; 4] = [
    guest::LDTR_SELECTOR,
    guest::LDTR_BASE,
    guest::LDTR_LIMIT,
    guest::LDTR_ACCESS_RIGHTS,
];
