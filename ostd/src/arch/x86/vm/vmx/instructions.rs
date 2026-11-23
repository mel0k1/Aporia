use crate::{Error, irq::DisabledLocalIrqGuard, prelude::*};

pub(super) unsafe fn vmxon(vmxon_region: Paddr, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    let failed: u8;

    unsafe {
        core::arch::asm!(
            "vmxon [{region}]",
            "setna {failed}",
            region = in(reg) &vmxon_region,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }

    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

pub(super) unsafe fn vmxoff(_irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    let failed: u8;

    unsafe {
        core::arch::asm!(
            "vmxoff",
            "setna {failed}",
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }

    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

#[cfg_attr(not(ktest), expect(dead_code))]
pub(super) unsafe fn vmptrst(_irq_guard: &DisabledLocalIrqGuard) -> u64 {
    let mut current_vmcs = 0_u64;

    unsafe {
        core::arch::asm!(
            "vmptrst [{current_vmcs_ptr}]",
            current_vmcs_ptr = in(reg) &mut current_vmcs,
            options(nostack)
        );
    }

    current_vmcs
}

pub(super) unsafe fn vmclear(vmcs_region: Paddr, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    let failed: u8;
    unsafe {
        core::arch::asm!(
            "vmclear [{region}]",
            "setna {failed}",
            region = in(reg) &vmcs_region,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }
    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

pub(super) unsafe fn vmptrld(vmcs_region: Paddr, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    let failed: u8;
    unsafe {
        core::arch::asm!(
            "vmptrld [{region}]",
            "setna {failed}",
            region = in(reg) &vmcs_region,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }
    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

pub(super) unsafe fn vmread(field: u32, _irq_guard: &DisabledLocalIrqGuard) -> Result<usize> {
    let value: usize;
    let failed: u8;
    unsafe {
        core::arch::asm!(
            "vmread {value}, {field}",
            "setna {failed}",
            value = out(reg) value,
            field = in(reg) field as usize,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }
    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(value)
}

pub(super) unsafe fn vmwrite(
    field: u32,
    value: usize,
    _irq_guard: &DisabledLocalIrqGuard,
) -> Result<()> {
    let failed: u8;
    unsafe {
        core::arch::asm!(
            "vmwrite {field}, {value}",
            "setna {failed}",
            field = in(reg) field as usize,
            value = in(reg) value,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }
    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}

pub(super) unsafe fn invept_all_contexts(_irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
    #[repr(C, align(16))]
    struct Descriptor {
        eptp: u64,
        reserved: u64,
    }

    let descriptor = Descriptor {
        eptp: 0,
        reserved: 0,
    };
    let failed: u8;
    unsafe {
        core::arch::asm!(
            "invept {typ}, [{descriptor}]",
            "setna {failed}",
            typ = in(reg) 2_u64,
            descriptor = in(reg) &descriptor,
            failed = out(reg_byte) failed,
            options(nostack)
        );
    }
    if failed != 0 {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}
