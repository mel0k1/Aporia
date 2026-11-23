use alloc::format;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ostd::{cpu::PinCurrentCpu, panic, task::disable_preempt, util::id_set::Id};

use super::Thread;
use crate::prelude::*;

static PANIC_ON_OOPS: AtomicBool = AtomicBool::new(true);

pub(crate) struct OopsInfo {
    pub message: String,
    #[expect(unused)]
    pub thread: Arc<Thread>,
}

pub(crate) fn catch_panics_as_oops<F, R>(f: F) -> Result<R, OopsInfo>
where
    F: FnOnce() -> R,
{
    let result = panic::catch_unwind(f);

    match result {
        Ok(result) => Ok(result),
        Err(err) => {
            let info = err.downcast::<OopsInfo>().unwrap();

            ostd::error!("Oops! {}", info.message);

            let count = OOPS_COUNT.fetch_add(1, Ordering::Relaxed);
            if count >= MAX_OOPS_COUNT {
                ostd::error!("Too many oops. The kernel panics.");
                panic::abort();
            }

            Err(*info)
        }
    }
}

const MAX_OOPS_COUNT: usize = 10_000;

static OOPS_COUNT: AtomicUsize = AtomicUsize::new(0);

#[ostd::panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let message = info.message();

    if let Some(thread) = Thread::current() {
        let panic_on_oops = PANIC_ON_OOPS.load(Ordering::Relaxed);
        if !panic_on_oops && info.can_unwind() {
            let message = if let Some(location) = info.location() {
                format!("{} at {}:{}", message, location.file(), location.line())
            } else {
                message.to_string()
            };
            panic::begin_panic(Box::new(OopsInfo { message, thread }));
        }
    }

    let preempt_guard = disable_preempt();
    let thread = Thread::current();
    let cpu = preempt_guard.current_cpu();

    if let Some(location) = info.location() {
        ostd::error!(
            "Uncaught panic:\n\t{}\n\tat {}:{}\n\ton CPU {} by thread {:?}",
            message,
            location.file(),
            location.line(),
            cpu.as_usize(),
            thread,
        );
    } else {
        ostd::error!(
            "Uncaught panic:\n\t{}\n\ton CPU {} by thread {:?}",
            message,
            cpu.as_usize(),
            thread,
        );
    }

    if info.can_unwind() {
        panic::print_stack_trace();
    } else {
        ostd::error!("Backtrace is disabled.");
    }

    panic::abort();
}
