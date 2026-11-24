use crate::{net::uts_ns::UtsField, prelude::*, syscall::SyscallReturn};

pub(super) fn sys_setdomainname(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let new_domain_name = read_uts_field(addr, len, ctx)?;

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    ns_proxy
        .uts_ns()
        .set_domainname(new_domain_name, ctx.posix_thread)?;

    Ok(SyscallReturn::Return(0))
}

/// Reads a UTS field from user space.
pub(super) fn read_uts_field(addr: Vaddr, len: usize, ctx: &Context) -> Result<UtsField> {
    if len > UtsField::MAX_BYTES {
        return Err(errno!(EINVAL, "the UTS name is too long"));
    }

    let user_space = ctx.user_space();
    let mut reader = user_space.reader(addr, len)?;
    let mut field = [0u8; UtsField::MAX_BYTES_WITH_NUL];

    if let Err((err, 0)) = reader.read_fallible(&mut VmWriter::from(field.as_mut_slice())) {
        return Err(err.into());
    }

    Ok(UtsField::from_bytes_until_nul(&field))
}
