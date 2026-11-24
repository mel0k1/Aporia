use core::time::Duration;

use aster_time::read_monotonic_time;
use aster_util::fixed_point::FixedU64;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, process::pid_table, sched::loadavg};

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct SysInfo {
    uptime: i64,
    loads: [u64; 3],
    totalram: u64,
    freeram: u64,
    sharedram: u64,
    bufferram: u64,
    totalswap: u64,
    freeswap: u64,
    procs: u16,
    totalhigh: u64,
    freehigh: u64,
    mem_unit: u32,
}

pub(super) fn sys_sysinfo(sysinfo_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    type SysinfoLoadAvg = FixedU64<16>;

    let loadavg = loadavg::get_loadavg();
    let info = SysInfo {
        uptime: read_monotonic_time().as_secs_round_up() as i64,
        loads: [
            SysinfoLoadAvg::from(loadavg[0]).raw(),
            SysinfoLoadAvg::from(loadavg[1]).raw(),
            SysinfoLoadAvg::from(loadavg[2]).raw(),
        ],
        totalram: crate::vm::mem_total() as u64,
        freeram: osdk_frame_allocator::load_total_free_size() as u64,
        procs: pid_table::pid_table_mut().process_count() as u16,

        mem_unit: 1,
        ..Default::default()
    };
    ctx.user_space().write_val(sysinfo_addr, &info)?;
    Ok(SyscallReturn::Return(0))
}

trait RoundUpSeconds {
    /// Returns the duration in seconds, rounding up a nonzero sub-second part.
    ///
    /// This matches the rounding used by Linux `sysinfo`.
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/kernel/sys.c#L2904-L2906>
    fn as_secs_round_up(&self) -> u64;
}

impl RoundUpSeconds for Duration {
    fn as_secs_round_up(&self) -> u64 {
        self.as_secs()
            .saturating_add(u64::from(self.subsec_nanos() != 0))
    }
}

#[cfg(ktest)]
mod tests {
    use core::time::Duration;

    use ostd::prelude::ktest;

    use super::RoundUpSeconds;

    #[ktest]
    fn as_secs_round_up() {
        assert_eq!(Duration::new(0, 0).as_secs_round_up(), 0);
        assert_eq!(Duration::new(42, 0).as_secs_round_up(), 42);
        assert_eq!(Duration::new(42, 1).as_secs_round_up(), 43);
        assert_eq!(Duration::new(42, 999_999_999).as_secs_round_up(), 43);
    }
}
