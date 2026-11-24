use core::sync::atomic::Ordering;

use super::{INIT_PROCESS_PID, Pid, Process, pid_table};
use crate::{
    events::IoEvents, fs::cgroupfs::CgroupMembership, prelude::*,
    process::signal::signals::kernel::KernelSignal,
};

/// Exits the current POSIX process.
///
/// This is for internal use. Do NOT call this directly. When the last thread in the process exits,
/// [`do_exit`] or [`do_exit_group`] will invoke this method automatically.
///
/// [`do_exit`]: crate::process::posix_thread::do_exit
/// [`do_exit_group`]: crate::process::posix_thread::do_exit_group
pub(super) fn exit_process(current_process: &Process) {
    current_process.status().set_vfork_child(false);

    drop_after!(current_process.lock_vmar().set_vmar(None));

    let children = move_children_to_reaper_process(current_process);
    send_parent_death_signal(&children);
    drop(children);

    current_process.status().set_zombie();

    current_process.pidfile_pollee.notify(IoEvents::IN);

    send_child_death_signal(current_process);

    let mut cgroup_guard = CgroupMembership::write_lock();
    cgroup_guard.move_process_to_root(current_process);
    drop(cgroup_guard);
}

/// Drops a value after releasing the lock or borrow guard.
///
/// For example, [`ThreadLocal::vmar`] needs to be borrowed during a context switch. This means that
/// the kernel may panic if we borrow it as mutable and tries to acquire a mutex while dropping the
/// last [`VmarHandle`]. This utility macro ensures that the value is dropped only after the borrow
/// guard has been released, preventing all these issues.
///
/// [`ThreadLocal::vmar`]: super::posix_thread::ThreadLocal::vmar
/// [`VmarHandle`]: crate::vm::vmar::VmarHandle
macro_rules! drop_after {
    ($expr:expr) => {
        let val = $expr;
        drop(val);
    };
}

pub(super) use drop_after;

/// Moves the children to a reaper process.
///
/// Returns the moved children. Note that no new processes can become this process's children after
/// this point, so the returned children reliably represent the set of children at the time the
/// process exits.
fn move_children_to_reaper_process(current_process: &Process) -> BTreeMap<Pid, Arc<Process>> {
    if current_process.is_init_process() {
        return BTreeMap::new();
    }

    while let Some(reaper_process) = find_reaper_process(current_process) {
        if let Ok(children) = move_process_children(current_process, &reaper_process) {
            reaper_process.children_wait_queue().wake_all();
            return children;
        }
    }

    let init_process = pid_table::pid_table_mut()
        .get_process(INIT_PROCESS_PID)
        .unwrap();
    let children = move_process_children(current_process, &init_process).unwrap();
    init_process.children_wait_queue().wake_all();

    children
}

/// Finds a reaper process for `current_process`.
///
/// If there is no reaper process for `current_process`, returns `None`.
fn find_reaper_process(current_process: &Process) -> Option<Arc<Process>> {
    let mut parent = current_process.parent().lock().process().upgrade().unwrap();

    loop {
        if parent.is_init_process() {
            return Some(parent);
        }

        if !parent.has_child_subreaper.load(Ordering::Acquire) {
            return None;
        }

        let is_reaper = parent.is_child_subreaper();
        let is_zombie = parent.status().is_zombie();
        if is_reaper && !is_zombie {
            return Some(parent);
        }

        let grandparent = parent.parent().lock().process().upgrade();
        if let Some(grandparent) = grandparent {
            parent = grandparent;
        } else {
            parent = current_process.parent().lock().process().upgrade().unwrap();
        }
    }
}

/// Moves the children of `current_process` to be the children of `reaper_process`.
///
/// Returns the moved children on success. Otherwise, if the `reaper_process` is zombie, returns
/// `Err(())`.
fn move_process_children(
    current_process: &Process,
    reaper_process: &Arc<Process>,
) -> Result<BTreeMap<Pid, Arc<Process>>, ()> {
    let mut reaper_process_children = reaper_process.children().lock();
    let Some(reaper_process_children) = reaper_process_children.as_mut() else {
        return Err(());
    };

    let mut current_children = current_process.children().lock();
    let children = current_children.take().unwrap();

    for child_process in children.values() {
        let mut parent = child_process.parent.lock();
        reaper_process_children.insert(child_process.pid(), child_process.clone());
        parent.set_process(reaper_process);
    }

    Ok(children)
}

/// Sends parent-death signals to the children.
fn send_parent_death_signal(children: &BTreeMap<Pid, Arc<Process>>) {
    for child in children.values() {
        let Some(signum) = child.parent_death_signal() else {
            continue;
        };

        let signal = Box::new(KernelSignal::new(signum));
        child.enqueue_signal(signal);
    }
}

/// Sends a child-death signal to the parent.
fn send_child_death_signal(current_process: &Process) {
    let Some(parent) = current_process.parent().lock().process().upgrade() else {
        return;
    };

    if let Some(signal) = current_process.exit_signal().map(KernelSignal::new) {
        parent.enqueue_signal(Box::new(signal));
    };
    parent.children_wait_queue().wake_all();
}
