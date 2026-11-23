use crate::early_println;

extern crate gimli;

#[linkage = "weak"]
#[unsafe(no_mangle)]
pub fn __ostd_panic_handler(info: &core::panic::PanicInfo) -> ! {
    let _irq_guard = crate::irq::disable_local();

    crate::cpu_local_cell! {
        static IN_PANIC: bool = false;
    }

    if IN_PANIC.load() {
        early_println!("The panic handler panicked {:#?}", info);
        abort();
    }

    IN_PANIC.store(true);

    early_println!("Non-resettable panic! {:#?}", info);

    print_stack_trace();
    abort();
}

pub fn abort() -> ! {
    crate::power::poweroff(crate::power::ExitCode::Failure);
}

#[derive(Debug)]
pub(crate) struct PanicGuard {
    _private: (),
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        early_println!("Panicked in `PanicGuard`, aborting the system");
        abort();
    }
}

impl PanicGuard {
    pub(crate) fn new() -> Self {
        PanicGuard { _private: () }
    }

    pub(crate) fn forget(self) {
        core::mem::forget(self);
    }
}

pub use unwinding::panic::{begin_panic, catch_unwind};

pub fn print_stack_trace() {
    use core::ffi::c_void;

    use gimli::Register;
    use unwinding::abi::{
        _Unwind_Backtrace, _Unwind_FindEnclosingFunction, _Unwind_GetGR, _Unwind_GetIP,
        UnwindContext, UnwindReasonCode,
    };

    use crate::{early_print, sync::SpinLock};

    static BACKTRACE_PRINT_LOCK: SpinLock<()> = SpinLock::new(());
    let _lock = BACKTRACE_PRINT_LOCK.lock();

    early_println!("Printing stack trace:");

    struct CallbackData {
        counter: usize,
    }
    extern "C" fn callback(unwind_ctx: &UnwindContext<'_>, arg: *mut c_void) -> UnwindReasonCode {
        let data = unsafe { &mut *(arg as *mut CallbackData) };
        data.counter += 1;
        let pc = _Unwind_GetIP(unwind_ctx);
        if pc > 0 {
            let fde_initial_address = _Unwind_FindEnclosingFunction(pc as *mut c_void) as usize;
            early_println!(
                "{:4}: fn {:#18x} - pc {:#18x} / registers:",
                data.counter,
                fde_initial_address,
                pc,
            );
        }
        for i in 0..8u16 {
            let reg_name = cfg_select! {
                target_arch = "x86_64" => gimli::X86_64::register_name(Register(i)),
                target_arch = "riscv64" => gimli::RiscV::register_name(Register(i)),
                target_arch = "loongarch64" => gimli::LoongArch::register_name(Register(i)),
                target_arch = "aarch64" => gimli::AArch64::register_name(Register(i)),
                _ => None,
            };
            let reg_val = _Unwind_GetGR(unwind_ctx, i as i32);
            if i.is_multiple_of(4) {
                early_print!("\n    ");
            }
            early_print!(" {} {:#18x};", reg_name.unwrap_or("unknown"), reg_val);
        }
        early_print!("\n\n");
        UnwindReasonCode::NO_REASON
    }

    let mut data = CallbackData { counter: 0 };
    _Unwind_Backtrace(callback, &mut data as *mut _ as _);
}
