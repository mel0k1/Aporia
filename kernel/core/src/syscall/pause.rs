use ostd::sync::Waiter;

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_pause(_ctx: &Context) -> Result<SyscallReturn> {
    let waiter = Waiter::new_pair().0;

    waiter.pause_until(|| None)?;

    unreachable!("[Internal Error] pause should always return EINTR");
}
