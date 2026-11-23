use crate::{
    arch::{cpu::context::UserContext, trap::TrapFrame},
    irq::DisabledLocalIrqGuard,
};

pub(crate) trait UserContextApiInternal {
    fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason;

    fn as_trap_frame(&self) -> TrapFrame;
}

pub trait UserContextApi {
    fn set_instruction_pointer(&mut self, ip: usize);

    fn instruction_pointer(&self) -> usize;

    fn set_stack_pointer(&mut self, sp: usize);

    fn stack_pointer(&self) -> usize;
}

pub struct UserMode {
    context: UserContext,
}

impl !Send for UserMode {}

impl UserMode {
    pub fn new(context: UserContext) -> Self {
        Self { context }
    }

    #[track_caller]
    pub fn execute<T: UserModeHooks>(&mut self, hooks: &T) -> ReturnReason {
        crate::task::atomic_mode::might_sleep();
        self.context.execute(hooks)
    }

    pub fn context(&self) -> &UserContext {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut UserContext {
        &mut self.context
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ReturnReason {
    UserSyscall,
    UserException,
    KernelEvent,
}

pub trait UserModeHooks {
    fn has_kernel_event(&self) -> bool {
        false
    }

    fn pre_user_run(&self, _guard: &DisabledLocalIrqGuard) {}
}

pub struct DummyUserHooks;

impl UserModeHooks for DummyUserHooks {}
