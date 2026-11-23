mod bottom_half;
mod guard;
mod level;
mod top_half;

pub use bottom_half::{register_bottom_half_handler_l1, register_bottom_half_handler_l2};
pub use guard::{DisabledLocalIrqGuard, disable_local};
pub use level::InterruptLevel;
pub use top_half::{IrqCallbackFunction, IrqLine};

use crate::{
    arch::{irq::HwIrqLine, trap::TrapFrame},
    cpu::PrivilegeLevel,
};

pub(crate) fn call_irq_callback_functions(
    trap_frame: &TrapFrame,
    hw_irq_line: &HwIrqLine,
    cpu_priv_at_irq: PrivilegeLevel,
) {
    level::enter(
        move || {
            top_half::process(trap_frame, hw_irq_line);
            bottom_half::process(hw_irq_line.irq_num());
        },
        cpu_priv_at_irq,
    );
}
