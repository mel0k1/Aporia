use core::sync::atomic::{Ordering, fence};

use ostd::cpu::CpuSet;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pid, Process},
};

const CMD_QUERY: u32 = 0;
const CMD_GLOBAL: u32 = 1;
const CMD_GLOBAL_EXPEDITED: u32 = 2;
const CMD_REGISTER_GLOBAL_EXPEDITED: u32 = 4;
const CMD_PRIVATE_EXPEDITED: u32 = 8;
const CMD_REGISTER_PRIVATE_EXPEDITED: u32 = 16;
const CMD_GET_REGISTRATIONS: u32 = 512;

const SUPPORTED_COMMANDS: u32 = CMD_GLOBAL
    | CMD_GLOBAL_EXPEDITED
    | CMD_REGISTER_GLOBAL_EXPEDITED
    | CMD_PRIVATE_EXPEDITED
    | CMD_REGISTER_PRIVATE_EXPEDITED
    | CMD_GET_REGISTRATIONS;

struct Registration {
    process: Weak<Process>,
    commands: u32,
}

static REGISTRATIONS: SpinLock<BTreeMap<Pid, Registration>> = SpinLock::new(BTreeMap::new());

fn run_full_barrier() {
    fence(Ordering::SeqCst);
}

fn execute_barrier() {
    let all_cpus = CpuSet::new_full();
    ostd::smp::inter_processor_call(&all_cpus, run_full_barrier).wait();
}

fn check_flags(flags: u32) -> Result<()> {
    if flags != 0 {
        return Err(errno!(EINVAL, "membarrier flags are not supported"));
    }
    Ok(())
}

fn registered_commands(pid: Pid) -> u32 {
    let mut registrations = REGISTRATIONS.lock();
    let Some(registration) = registrations.get(&pid) else {
        return 0;
    };
    let alive = registration.process.upgrade().is_some();
    let commands = registration.commands;
    if !alive {
        registrations.remove(&pid);
        return 0;
    }
    commands
}

fn register_command(pid: Pid, command: u32, process: &Arc<Process>) {
    let mut registrations = REGISTRATIONS.lock();
    registrations.retain(|_, registration| registration.process.upgrade().is_some());
    registrations
        .entry(pid)
        .or_insert_with(|| Registration {
            process: Arc::downgrade(process),
            commands: 0,
        })
        .commands |= command;
}

pub(super) fn sys_membarrier(cmd: u32, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let pid = ctx.process.pid();
    debug!("pid = {}, cmd = {}", pid, cmd);

    match cmd {
        CMD_QUERY => Ok(SyscallReturn::Return(SUPPORTED_COMMANDS as _)),
        CMD_GLOBAL => {
            check_flags(flags)?;
            execute_barrier();
            Ok(SyscallReturn::Return(0))
        }
        CMD_REGISTER_GLOBAL_EXPEDITED | CMD_REGISTER_PRIVATE_EXPEDITED => {
            check_flags(flags)?;
            let command = match cmd {
                CMD_REGISTER_GLOBAL_EXPEDITED => CMD_GLOBAL_EXPEDITED,
                _ => CMD_PRIVATE_EXPEDITED,
            };
            register_command(pid, command, &ctx.process);
            Ok(SyscallReturn::Return(0))
        }
        CMD_GLOBAL_EXPEDITED | CMD_PRIVATE_EXPEDITED => {
            check_flags(flags)?;
            if registered_commands(pid) & cmd == 0 {
                return Err(errno!(
                    EINVAL,
                    "the membarrier command requires registration"
                ));
            }
            execute_barrier();
            Ok(SyscallReturn::Return(0))
        }
        CMD_GET_REGISTRATIONS => {
            check_flags(flags)?;
            Ok(SyscallReturn::Return(registered_commands(pid) as _))
        }
        _ => Err(errno!(EINVAL, "unsupported membarrier command")),
    }
}
