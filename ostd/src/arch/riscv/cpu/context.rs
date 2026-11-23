use alloc::boxed::Box;
use core::{arch::global_asm, fmt::Debug};

use ostd_pod::IntoBytes;
use riscv::{
    interrupt::supervisor::{Exception, Interrupt},
    register::scause::Trap,
};

use crate::{
    arch::{
        cpu::extension::{IsaExtensions, has_extensions},
        trap::{RawUserContext, SSTATUS_FS_MASK, TrapFrame, handle_irq},
    },
    cpu::PrivilegeLevel,
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
#[derive(Clone, Copy, Debug, Default)]
pub struct GeneralRegs {
    pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum CpuException {
    InstructionMisaligned,
    InstructionFault,
    IllegalInstruction(FaultInstruction),
    Breakpoint,
    LoadMisaligned(FaultAddress),
    LoadFault(FaultAddress),
    StoreMisaligned(FaultAddress),
    StoreFault(FaultAddress),
    UserEnvCall,
    SupervisorEnvCall,
    InstructionPageFault(FaultAddress),
    LoadPageFault(FaultAddress),
    StorePageFault(FaultAddress),
    Unknown,
}

impl CpuException {
    pub(in crate::arch) fn new(raw_exception: Exception, stval: usize) -> Self {
        use Exception::*;

        match raw_exception {
            InstructionMisaligned => Self::InstructionMisaligned,
            InstructionFault => Self::InstructionFault,
            IllegalInstruction => Self::IllegalInstruction({
                if stval & 0x3 == 0x3 {
                    FaultInstruction::Normal(stval as u32)
                } else {
                    FaultInstruction::Compressed(stval as u16)
                }
            }),
            Breakpoint => Self::Breakpoint,
            LoadMisaligned => Self::LoadMisaligned(stval),
            LoadFault => Self::LoadFault(stval),
            StoreMisaligned => Self::StoreMisaligned(stval),
            StoreFault => Self::StoreFault(stval),
            UserEnvCall => Self::UserEnvCall,
            SupervisorEnvCall => Self::SupervisorEnvCall,
            InstructionPageFault => Self::InstructionPageFault(stval),
            LoadPageFault => Self::LoadPageFault(stval),
            StorePageFault => Self::StorePageFault(stval),
        }
    }
}

pub type FaultAddress = usize;

#[derive(Clone, Copy, Debug)]
pub enum FaultInstruction {
    Normal(u32),
    Compressed(u16),
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
        self.tp()
    }

    pub fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls)
    }
}

impl UserContextApiInternal for UserContext {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        loop {
            crate::task::scheduler::might_preempt();

            let irq_guard = crate::irq::disable_local();
            hooks.pre_user_run(&irq_guard);
            self.user_context.run(&irq_guard);

            let scause = riscv::register::scause::read();
            let Ok(cause) = Trap::<Interrupt, Exception>::try_from(scause.cause()) else {
                match scause.cause() {
                    Trap::Interrupt(i) => {
                        panic!(
                            "Cannot handle unknown interrupt: {:#x?}; trapframe: {:#x?}",
                            i,
                            self.as_trap_frame()
                        );
                    }
                    Trap::Exception(e) => {
                        crate::info!("Unknown CPU exception in user mode: {:#x?}", e);
                        self.exception = Some(CpuException::Unknown);
                        break ReturnReason::UserException;
                    }
                }
            };
            match cause {
                Trap::Interrupt(interrupt) => {
                    handle_irq(&self.as_trap_frame(), interrupt, PrivilegeLevel::User);
                    drop(irq_guard);
                }
                Trap::Exception(Exception::UserEnvCall) => {
                    drop(irq_guard);
                    self.user_context.sepc += 4;
                    break ReturnReason::UserSyscall;
                }
                Trap::Exception(raw_exception) => {
                    let stval = riscv::register::stval::read();
                    drop(irq_guard);
                    let exception = CpuException::new(raw_exception, stval);
                    self.exception = Some(exception);
                    break ReturnReason::UserException;
                }
            }

            if hooks.has_kernel_event() {
                break ReturnReason::KernelEvent;
            }
        }
    }

    fn as_trap_frame(&self) -> TrapFrame {
        TrapFrame {
            general: self.user_context.general,
            sstatus: self.user_context.sstatus,
            sepc: self.user_context.sepc,
        }
    }
}

impl UserContextApi for UserContext {
    fn instruction_pointer(&self) -> usize {
        self.user_context.sepc
    }

    fn set_instruction_pointer(&mut self, ip: usize) {
        self.user_context.sepc = ip;
    }

    fn stack_pointer(&self) -> usize {
        self.sp()
    }

    fn set_stack_pointer(&mut self, sp: usize) {
        self.set_sp(sp);
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
    [ra, set_ra],
    [sp, set_sp],
    [gp, set_gp],
    [tp, set_tp],
    [t0, set_t0],
    [t1, set_t1],
    [t2, set_t2],
    [s0, set_s0],
    [s1, set_s1],
    [a0, set_a0],
    [a1, set_a1],
    [a2, set_a2],
    [a3, set_a3],
    [a4, set_a4],
    [a5, set_a5],
    [a6, set_a6],
    [a7, set_a7],
    [s2, set_s2],
    [s3, set_s3],
    [s4, set_s4],
    [s5, set_s5],
    [s6, set_s6],
    [s7, set_s7],
    [s8, set_s8],
    [s9, set_s9],
    [s10, set_s10],
    [s11, set_s11],
    [t3, set_t3],
    [t4, set_t4],
    [t5, set_t5],
    [t6, set_t6]
);

#[derive(Clone, Debug)]
pub enum FpuContext {
    F(Box<FFpuContext>),
    D(Box<DFpuContext>),
    Q(Box<QFpuContext>),
    None,
}

impl FpuContext {
    pub fn new() -> Self {
        if has_extensions(IsaExtensions::Q) {
            Self::Q(Box::default())
        } else if has_extensions(IsaExtensions::D) {
            Self::D(Box::default())
        } else if has_extensions(IsaExtensions::F) {
            Self::F(Box::default())
        } else {
            Self::None
        }
    }

    pub fn save(&mut self) {
        match self {
            Self::F(ctx) => ctx.save(),
            Self::D(ctx) => ctx.save(),
            Self::Q(ctx) => ctx.save(),
            Self::None => {}
        }
    }

    pub fn load(&self) {
        match self {
            Self::F(ctx) => ctx.load(),
            Self::D(ctx) => ctx.load(),
            Self::Q(ctx) => ctx.load(),
            Self::None => {}
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::F(ctx) => ctx.as_bytes(),
            Self::D(ctx) => ctx.as_bytes(),
            Self::Q(ctx) => ctx.as_bytes(),
            Self::None => &[],
        }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        match self {
            Self::F(ctx) => ctx.as_mut_bytes(),
            Self::D(ctx) => ctx.as_mut_bytes(),
            Self::Q(ctx) => ctx.as_mut_bytes(),
            Self::None => &mut [],
        }
    }
}

impl Default for FpuContext {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct FFpuContext {
    f: [u32; 32],
    fcsr: u32,
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct DFpuContext {
    f: [u64; 32],
    fcsr: u32,
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct QFpuContext {
    f: [u64; 64],
    fcsr: u32,
}

impl Default for QFpuContext {
    fn default() -> Self {
        Self {
            f: [0; 64],
            __pad1: [0; _],
            fcsr: 0,
            __pad2: [0; _],
        }
    }
}

impl FFpuContext {
    fn save(&mut self) {
        unsafe { save_fpu_context_f(self as *mut _) };
    }

    fn load(&self) {
        unsafe { load_fpu_context_f(self as *const _) };
    }
}

impl DFpuContext {
    fn save(&mut self) {
        unsafe { save_fpu_context_d(self as *mut _) };
    }

    fn load(&self) {
        unsafe { load_fpu_context_d(self as *const _) };
    }
}

impl QFpuContext {
    fn save(&mut self) {
        unsafe { save_fpu_context_q(self as *mut _) };
    }

    fn load(&self) {
        unsafe { load_fpu_context_q(self as *const _) };
    }
}

global_asm!(include_str!("fpu.S"), SSTATUS_FS_MASK = const SSTATUS_FS_MASK);

unsafe extern "C" {
    unsafe fn save_fpu_context_f(ctx: *mut FFpuContext);
    unsafe fn load_fpu_context_f(ctx: *const FFpuContext);
    unsafe fn save_fpu_context_d(ctx: *mut DFpuContext);
    unsafe fn load_fpu_context_d(ctx: *const DFpuContext);
    unsafe fn save_fpu_context_q(ctx: *mut QFpuContext);
    unsafe fn load_fpu_context_q(ctx: *const QFpuContext);
}
