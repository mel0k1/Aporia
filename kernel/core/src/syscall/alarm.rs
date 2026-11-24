use core::time::Duration;

use super::SyscallReturn;
use crate::{
    prelude::*,
    time::{NSEC_PER_SEC, timer::Timeout},
};

pub(super) fn sys_alarm(seconds: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("seconds = {}", seconds);

    let alarm_timer = ctx.process.timer_manager().alarm_timer();
    let mut timer_guard = alarm_timer.lock();

    let remaining = timer_guard.remain();
    let mut remaining_secs = remaining.as_secs();

    if (remaining_secs == 0 && remaining.subsec_nanos() > 0)
        || remaining.subsec_nanos() as i64 >= NSEC_PER_SEC / 2
    {
        remaining_secs += 1;
    }

    if seconds == 0 {
        timer_guard.cancel();
        return Ok(SyscallReturn::Return(remaining_secs as _));
    }

    timer_guard.set_timeout(Timeout::After(Duration::from_secs(seconds as u64)));

    Ok(SyscallReturn::Return(remaining_secs as _))
}
