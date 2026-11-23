#[expect(clippy::module_inception)]
mod trap;

pub(super) use trap::RawUserContext;
pub use trap::TrapFrame;

use crate::{
    arch::{
        cpu::context::{CpuException, CpuTrap},
        irq::{IRQ_CHIP, PSTATE_I, disable_local, enable_local},
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
};

pub(crate) unsafe fn init_on_cpu() {
    unsafe {
        trap::init_on_cpu();
    }
}

#[unsafe(no_mangle)]
extern "C" fn trap_handler(f: &mut TrapFrame) {
    fn enable_local_if(cond: bool) {
        if cond {
            enable_local();
        }
    }

    fn disable_local_if(cond: bool) {
        if cond {
            disable_local();
        }
    }

    let was_irq_enabled = f.spsr & PSTATE_I == 0;

    let trap = CpuTrap::new(f.trap_num);
    match trap {
        Some(CpuTrap::Exception(data_abort @ CpuException::DataAbort { address, .. })) => {
            enable_local_if(was_irq_enabled);
            crate::mm::fault::handle_user_page_fault(f, &data_abort, address);
            disable_local_if(was_irq_enabled);
        }
        Some(CpuTrap::Interrupt) => {
            let irq_chip = IRQ_CHIP.get().unwrap();
            while let Some(hw_irq_line) = irq_chip.claim_interrupt() {
                call_irq_callback_functions(f, &hw_irq_line, PrivilegeLevel::Kernel);
            }
        }
        _ => panic!(
            "Cannot handle kernel CPU exception: {:?}; trapframe: {:#?}",
            trap, f
        ),
    }
}
