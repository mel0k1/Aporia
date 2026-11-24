//! The timer support.

use super::trap::TrapFrame;

#[expect(dead_code)]
fn timer_callback(trapframe: &TrapFrame) {
    crate::timer::call_timer_callback_functions(trapframe);
}
