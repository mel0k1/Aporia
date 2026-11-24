use core::time::Duration;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, process::signal::c_types::clock_t, time::clocks::MonotonicClock};

const CLK_TCK: u128 = 100;

pub(super) fn sys_times(tms_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("tms_addr = {:#x}", tms_addr);

    let process = ctx.process.as_ref();

    let utime = process.prof_clock().user_clock().read_time();
    let stime = process.prof_clock().kernel_clock().read_time();
    let (cutime, cstime) = ctx.process.reaped_children_stats().lock().get();

    if tms_addr != 0 {
        let tms = tms_t {
            tms_utime: duration_to_clock_t(utime),
            tms_stime: duration_to_clock_t(stime),
            tms_cutime: duration_to_clock_t(cutime),
            tms_cstime: duration_to_clock_t(cstime),
        };
        ctx.user_space().write_val(tms_addr, &tms)?;
    }

    let elapsed = duration_to_clock_t(MonotonicClock::get().read_time());

    Ok(SyscallReturn::Return(elapsed as _))
}

fn duration_to_clock_t(duration: Duration) -> clock_t {
    let nanos = duration.as_nanos() * CLK_TCK;
    (nanos / 1_000_000_000) as clock_t
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct tms_t {
    tms_utime: clock_t,
    tms_stime: clock_t,
    tms_cutime: clock_t,
    tms_cstime: clock_t,
}
