use super::SyscallReturn;
use crate::{device, prelude::*};

pub(super) fn sys_getrandom(
    buf: Vaddr,
    count: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = GetRandomFlags::from_bits(flags).ok_or_else(|| errno!(EINVAL, "invalid flags"))?;
    debug!(
        "buf = 0x{:x}, count = 0x{:x}, flags = {:?}",
        buf, count, flags
    );

    if flags.contains(GetRandomFlags::GRND_INSECURE | GetRandomFlags::GRND_RANDOM) {
        return Err(errno!(
            EINVAL,
            "requesting insecure and blocking randomness makes no sense"
        ));
    }

    let user_space = ctx.user_space();
    let mut writer = user_space.writer(buf, count)?;
    let read_len = if flags.contains(GetRandomFlags::GRND_RANDOM) {
        device::getrandom(&mut writer)?
    } else {
        device::geturandom(&mut writer)?
    };
    Ok(SyscallReturn::Return(read_len as isize))
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct GetRandomFlags: u32 {
        const GRND_NONBLOCK = 0x0001;
        const GRND_RANDOM   = 0x0002;
        const GRND_INSECURE = 0x0004;
    }
}
