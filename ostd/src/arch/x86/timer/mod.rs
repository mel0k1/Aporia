//! The timer support.

macro_rules! __log_prefix {
    () => {
        "timer: "
    };
}

mod apic;
mod hpet;
pub(in crate::arch) mod pit;

use spin::Once;

use super::trap::TrapFrame;
use crate::irq::IrqLine;

static TIMER_IRQ: Once<IrqLine> = Once::new();

/// Initializes the timer state and enable timer interrupts on BSP.
pub(super) fn init_on_bsp() {
    let mut timer_irq = apic::init_on_bsp();

    timer_irq.on_active(timer_callback);

    TIMER_IRQ.call_once(|| timer_irq);
}

/// Enables timer interrupt on this AP.
pub(super) fn init_on_ap() {
    apic::init_on_ap(TIMER_IRQ.get().unwrap());
}

fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);

    apic::timer_callback();
}
