use spin::Once;

use super::{DisabledLocalIrqGuard, InterruptLevel, disable_local};
use crate::task::disable_preempt;

pub fn register_bottom_half_handler_l1(
    func: fn(DisabledLocalIrqGuard, u8) -> DisabledLocalIrqGuard,
) {
    BOTTOM_HALF_HANDLER_L1.call_once(|| func);
}

pub fn register_bottom_half_handler_l2(func: fn(u8)) {
    BOTTOM_HALF_HANDLER_L2.call_once(|| func);
}

static BOTTOM_HALF_HANDLER_L1: Once<fn(DisabledLocalIrqGuard, u8) -> DisabledLocalIrqGuard> =
    Once::new();
static BOTTOM_HALF_HANDLER_L2: Once<fn(u8)> = Once::new();

pub(super) fn process(irq_num: u8) {
    match InterruptLevel::current() {
        InterruptLevel::L1(_) => process_l1(irq_num),
        InterruptLevel::L2 => process_l2(irq_num),
        _ => unreachable!("this function must have been call in interrupt context"),
    }
}

fn process_l1(irq_num: u8) {
    let Some(handler) = BOTTOM_HALF_HANDLER_L1.get() else {
        return;
    };

    let preempt_guard = disable_preempt();
    crate::arch::irq::enable_local();

    let irq_guard = disable_local();
    let irq_guard = handler(irq_guard, irq_num);

    core::mem::forget(irq_guard);
    drop(preempt_guard);
}

fn process_l2(irq_num: u8) {
    let Some(handler) = BOTTOM_HALF_HANDLER_L2.get() else {
        return;
    };

    handler(irq_num);
}
