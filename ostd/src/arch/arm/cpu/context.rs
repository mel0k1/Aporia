use alloc::boxed::Box;
use core::{arch::asm, fmt::Debug};

use zerocopy::IntoBytes;

use crate::{
    arch::{
        irq::IRQ_CHIP,
        trap::{RawUserContext, TrapFrame},
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
    user::{ReturnReason, UserContextApi, UserContextApiInternal, UserModeHooks},
};

#[repr(C)]
#[derive(Clone, Debug, Default)]
pub struct UserContext {
    user_context: RawUserContext,
    exception: Option<CpuException>,
}

#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GeneralRegs {
    pub x1: usize,
    pub x2: usize,
    pub x3: usize,
    pub x4: usize,
    pub x5: usize,
    pub x6: usize,
    pub x7: usize,
    pub x8: usize,
    pub x9: usize,
    pub x10: usize,
    pub x11: usize,
    pub x12: usize,
    pub x13: usize,
    pub x14: usize,
    pub x15: usize,
    pub x16: usize,
    pub x17: usize,
    pub x18: usize,
    pub x19: usize,
    pub x20: usize,
    pub x21: usize,
    pub x22: usize,
    pub x23: usize,
    pub x24: usize,
    pub x25: usize,
    pub x26: usize,
    pub x27: usize,
    pub x28: usize,
    pub x29: usize,
    pub __reserved: usize,
    pub x30: usize,
    pub x0: usize,
}

#[derive(Clone, Debug)]
pub enum CpuException {
    Unknown,
    WfiInstruction,
    FpuInstruction,
    IllegalState,
    SvcInstruction,
    SystemInstruction,
    InstructionAbort { address: usize },
    PcAlignmentFault,
    DataAbort { is_write: bool, address: usize },
    SpAlignmentFault,
    FpuException,
    SErrorInterrupt,
    Breakpoint,
    SoftwareStep,
    Watchpoint,
    BrkInstruction,
}

#[derive(Clone, Debug)]
pub(in crate::arch) enum CpuTrap {
    Exception(CpuException),
    Interrupt,
    FastInterrupt,
    SError,
}

impl CpuTrap {
    pub(in crate::arch) fn new(trap_num: usize) -> Option<Self> {
        match trap_num >> 16 {
            0 => (),
            1 => return Some(Self::Interrupt),
            2 => return Some(Self::FastInterrupt),
            3 => return Some(Self::SError),

            _ => return None,
        }

        let esr_el1: usize;
        unsafe { asm!("mrs {}, esr_el1", out(reg) esr_el1) };

        fn fault_address() -> usize {
            let far_el1;
            unsafe { asm!("mrs {}, far_el1", out(reg) far_el1) };
            far_el1
        }

        const ESR_WNR: usize = 1 << 6;

        let exception = match esr_el1 >> 26 {
            0b000001 => CpuException::WfiInstruction,
            0b000111 => CpuException::FpuInstruction,
            0b001110 => CpuException::IllegalState,
            0b010101 => CpuException::SvcInstruction,
            0b011000 => CpuException::SystemInstruction,
            0b100000 | 0b100001 => CpuException::InstructionAbort {
                address: fault_address(),
            },
            0b100010 => CpuException::PcAlignmentFault,
            0b100100 | 0b100101 => CpuException::DataAbort {
                is_write: esr_el1 & ESR_WNR != 0,
                address: fault_address(),
            },
            0b100110 => CpuException::SpAlignmentFault,
            0b101100 => CpuException::FpuException,
            0b101111 => CpuException::SErrorInterrupt,
            0b110000 | 0b110001 => CpuException::Breakpoint,
            0b110010 | 0b110011 => CpuException::SoftwareStep,
            0b110100 | 0b110101 => CpuException::Watchpoint,
            0b111100 => CpuException::BrkInstruction,

            0b000000 => CpuException::Unknown,
            _ => CpuException::Unknown,
        };
        Some(Self::Exception(exception))
    }
}

impl UserContext {
    pub fn general_regs(&self) -> &GeneralRegs {
        &self.user_context.general
    }

    pub fn general_regs_mut(&mut self) -> &mut GeneralRegs {
        &mut self.user_context.general
    }

    pub fn take_exception(&mut self) -> Option<CpuException> {
        self.exception.take()
    }

    pub fn tls_pointer(&self) -> usize {
        self.user_context.tpidr
    }

    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.user_context.tpidr = tls;
    }

    pub fn process_state(&self) -> usize {
        self.user_context.spsr
    }

    pub fn set_process_state(&mut self, pstate: usize) {
        const USER_MODIFIABLE_PSTATE: usize = (0b1111 << 28) | (1 << 21) | (1 << 20);

        self.user_context.spsr =
            (self.user_context.spsr & !USER_MODIFIABLE_PSTATE) | (pstate & USER_MODIFIABLE_PSTATE);
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        loop {
            crate::task::scheduler::might_preempt();

            let irq_guard = crate::irq::disable_local();
            hooks.pre_user_run(&irq_guard);
            self.user_context.run(&irq_guard);

            let trap = CpuTrap::new(self.user_context.trap_num);
            match trap {
                Some(CpuTrap::Exception(CpuException::SvcInstruction)) => {
                    drop(irq_guard);
                    break ReturnReason::UserSyscall;
                }
                Some(CpuTrap::Exception(exception)) => {
                    drop(irq_guard);
                    self.exception = Some(exception);
                    break ReturnReason::UserException;
                }
                Some(CpuTrap::Interrupt) => {
                    let irq_chip = IRQ_CHIP.get().unwrap();
                    while let Some(hw_irq_line) = irq_chip.claim_interrupt() {
                        call_irq_callback_functions(
                            &self.as_trap_frame(),
                            &hw_irq_line,
                            PrivilegeLevel::User,
                        );
                    }
                    drop(irq_guard);
                }
                _ => panic!(
                    "Cannot handle user CPU exception: {:?}; trapframe: {:#?}",
                    trap,
                    self.as_trap_frame()
                ),
            }

            if hooks.has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            trap_num: self.user_context.trap_num,
            __reserved: self.user_context.__reserved,
            elr: self.user_context.elr,
            spsr: self.user_context.spsr,
            sp: self.user_context.sp,
            tpidr: self.user_context.tpidr,
            general: self.user_context.general,
        }
    }
}

impl UserContextApi for UserContext {
    fn instruction_pointer(&self) -> usize {
        self.user_context.elr
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.elr = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.user_context.sp
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.user_context.sp = sp;
    }
}

macro_rules! cpu_context_impl_getter_setter {
    ( $( [ $field: ident, $setter_name: ident] ),*) => {
        impl UserContext {
            $(
                #[doc = concat!("Gets the value of ", stringify!($field))]
                #[inline(always)]
                pub fn $field(&self) -> usize {
                    self.user_context.general.$field
                }

                #[doc = concat!("Sets the value of ", stringify!($field))]
                #[inline(always)]
                pub fn $setter_name(&mut self, $field: usize) {
                    self.user_context.general.$field = $field;
                }
            )*
        }
    };
}

cpu_context_impl_getter_setter!(
    [x0, set_x0],
    [x1, set_x1],
    [x2, set_x2],
    [x3, set_x3],
    [x4, set_x4],
    [x5, set_x5],
    [x6, set_x6],
    [x7, set_x7],
    [x8, set_x8],
    [x9, set_x9],
    [x10, set_x10],
    [x11, set_x11],
    [x12, set_x12],
    [x13, set_x13],
    [x14, set_x14],
    [x15, set_x15],
    [x16, set_x16],
    [x17, set_x17],
    [x18, set_x18],
    [x19, set_x19],
    [x20, set_x20],
    [x21, set_x21],
    [x22, set_x22],
    [x23, set_x23],
    [x24, set_x24],
    [x25, set_x25],
    [x26, set_x26],
    [x27, set_x27],
    [x28, set_x28],
    [x29, set_x29],
    [x30, set_x30]
);

#[derive(Clone, Debug)]
pub struct FpuContext {
    fpsimd: Box<FpSimdContext>,
}

impl FpuContext {
    const FPSIMD_MAGIC: u32 = 0x46508001;

    pub fn new() -> Self {
        let mut fpsimd = Box::new(FpSimdContext::default());

        fpsimd.magic = Self::FPSIMD_MAGIC;
        fpsimd.size = size_of::<FpSimdContext>() as u32;

        Self { fpsimd }
    }

    pub fn save(&mut self) {
        unsafe { save_fpsimd_context(self.fpsimd.as_mut()) };
    }

    pub fn load(&self) {
        unsafe { load_fpsimd_context(self.fpsimd.as_ref()) };
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.fpsimd.as_bytes()
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.fpsimd.as_mut_bytes()
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct FpSimdContext {
    magic: u32,
    size: u32,

    fpsr: u32,
    fpcr: u32,
    vregs: [u128; 32],
}

core::arch::global_asm!(include_str!("fpu.S"));

unsafe extern "C" {
    unsafe fn save_fpsimd_context(ctx: *mut FpSimdContext);
    unsafe fn load_fpsimd_context(ctx: *const FpSimdContext);
}
