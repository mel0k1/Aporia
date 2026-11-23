use super::SyscallReturn;
use crate::{prelude::*, process::posix_thread::Personality};

pub(super) fn sys_personality(personality: u32, ctx: &Context) -> Result<SyscallReturn> {
    let old_personality = ctx.posix_thread.personality() as isize;
    if personality == GET_PERSONALITY {
        return Ok(SyscallReturn::Return(old_personality));
    }

    if Personality::from_bits_truncate(personality).contains(Personality::ADDR_NO_RANDOMIZE) {
        warn!("`personality(ADDR_NO_RANDOMIZE)` is accepted, but still does not disable ASLR");
    }

    ctx.posix_thread.set_personality(personality);
    Ok(SyscallReturn::Return(old_personality))
}

const GET_PERSONALITY: u32 = 0xffffffff;
