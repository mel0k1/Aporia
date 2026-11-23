pub mod atomic_mode;
mod kernel_stack;
mod preempt;
mod processor;
pub mod scheduler;
mod utils;

use core::{
    any::Any,
    borrow::Borrow,
    cell::{Cell, SyncUnsafeCell},
    ops::Deref,
    ptr::NonNull,
    sync::atomic::AtomicBool,
};

use kernel_stack::KernelStack;
use processor::current_task;
use spin::Once;
use utils::ForceSync;

pub use self::{
    preempt::{DisabledPreemptGuard, disable_preempt, halt_cpu},
    scheduler::info::{AtomicCpuId, TaskScheduleInfo},
};
use crate::{
    arch::task::TaskContext,
    irq::{DisabledLocalIrqGuard, InterruptLevel},
    prelude::*,
};

static PRE_SCHEDULE_HANDLER: Once<fn(&DisabledLocalIrqGuard)> = Once::new();

static POST_SCHEDULE_HANDLER: Once<fn()> = Once::new();

pub fn inject_pre_schedule_handler(handler: fn(&DisabledLocalIrqGuard)) {
    PRE_SCHEDULE_HANDLER.call_once(|| handler);
}

pub fn inject_post_schedule_handler(handler: fn()) {
    POST_SCHEDULE_HANDLER.call_once(|| handler);
}

#[derive(Debug)]
pub struct Task {
    #[expect(clippy::type_complexity)]
    func: ForceSync<Cell<Option<Box<dyn FnOnce() + Send>>>>,

    data: Box<dyn Any + Send + Sync>,
    local_data: ForceSync<Box<dyn Any + Send>>,

    ctx: SyncUnsafeCell<TaskContext>,
    kstack: KernelStack,

    switched_to_cpu: AtomicBool,

    schedule_info: TaskScheduleInfo,
}

impl Task {
    pub fn current() -> Option<CurrentTask> {
        let current_task = current_task()?;

        Some(unsafe { CurrentTask::new(current_task) })
    }

    pub(super) fn ctx(&self) -> &SyncUnsafeCell<TaskContext> {
        &self.ctx
    }

    pub fn need_yield() -> bool {
        preempt::cpu_local::need_preempt()
    }

    #[track_caller]
    pub fn yield_now() {
        scheduler::yield_now()
    }

    #[track_caller]
    pub fn run(self: &Arc<Self>) {
        scheduler::run_new_task(self.clone());
    }

    pub fn data(&self) -> &Box<dyn Any + Send + Sync> {
        &self.data
    }

    pub fn schedule_info(&self) -> &TaskScheduleInfo {
        &self.schedule_info
    }
}

pub struct TaskOptions {
    func: Option<Box<dyn FnOnce() + Send>>,
    data: Option<Box<dyn Any + Send + Sync>>,
    local_data: Option<Box<dyn Any + Send>>,
}

impl TaskOptions {
    pub fn new<F>(func: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            func: Some(Box::new(func)),
            data: None,
            local_data: None,
        }
    }

    pub fn func<F>(mut self, func: F) -> Self
    where
        F: Fn() + Send + 'static,
    {
        self.func = Some(Box::new(func));
        self
    }

    pub fn data<T>(mut self, data: T) -> Self
    where
        T: Any + Send + Sync,
    {
        self.data = Some(Box::new(data));
        self
    }

    pub fn local_data<T>(mut self, data: T) -> Self
    where
        T: Any + Send,
    {
        self.local_data = Some(Box::new(data));
        self
    }

    pub fn build(self) -> Result<Task> {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn kernel_task_entry() -> ! {
            unsafe { processor::after_switching_to() };

            let current_task = Task::current()
                .expect("no current task, it should have current task in kernel task entry");

            let task_func = unsafe { current_task.func.get() };
            let task_func = task_func
                .take()
                .expect("task function is `None` when trying to run");
            task_func();

            scheduler::exit_current();
        }

        let kstack = KernelStack::new_with_guard_page()?;

        let mut ctx = TaskContext::new();
        ctx.set_instruction_pointer(
            crate::arch::task::kernel_task_entry_wrapper as *const () as usize,
        );
        ctx.set_stack_pointer(kstack.end_vaddr() - 16);

        let new_task = Task {
            func: ForceSync::new(Cell::new(self.func)),
            data: self.data.unwrap_or_else(|| Box::new(())),
            local_data: ForceSync::new(self.local_data.unwrap_or_else(|| Box::new(()))),
            ctx: SyncUnsafeCell::new(ctx),
            kstack,
            switched_to_cpu: AtomicBool::new(false),
            schedule_info: TaskScheduleInfo {
                cpu: AtomicCpuId::default(),
            },
        };

        Ok(new_task)
    }

    #[track_caller]
    pub fn spawn(self) -> Result<Arc<Task>> {
        let task = Arc::new(self.build()?);
        task.run();
        Ok(task)
    }
}

#[derive(Debug)]
pub struct CurrentTask(NonNull<Task>);

impl !Send for CurrentTask {}
impl !Sync for CurrentTask {}

impl CurrentTask {
    unsafe fn new(task: NonNull<Task>) -> Self {
        Self(task)
    }

    pub fn local_data(&self) -> &(dyn Any + Send) {
        assert!(InterruptLevel::current().is_task_context());

        let local_data = &self.local_data;

        &**unsafe { local_data.get() }
    }

    pub fn cloned(&self) -> Arc<Task> {
        let ptr = self.0.as_ptr();

        unsafe { Arc::increment_strong_count(ptr) };

        unsafe { Arc::from_raw(ptr) }
    }
}

impl Deref for CurrentTask {
    type Target = Task;

    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl AsRef<Task> for CurrentTask {
    fn as_ref(&self) -> &Task {
        self
    }
}

impl Borrow<Task> for CurrentTask {
    fn borrow(&self) -> &Task {
        self
    }
}

pub(crate) trait TaskContextApi {
    fn set_instruction_pointer(&mut self, ip: usize);

    fn set_stack_pointer(&mut self, sp: usize);
}

#[cfg(ktest)]
mod test {
    use crate::prelude::*;

    #[ktest]
    fn create_task() {
        #[expect(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let task = Arc::new(
            crate::task::TaskOptions::new(task)
                .data(())
                .build()
                .unwrap(),
        );
        task.run();
    }

    #[ktest]
    fn spawn_task() {
        #[expect(clippy::eq_op)]
        let task = || {
            assert_eq!(1, 1);
        };
        let _ = crate::task::TaskOptions::new(task).data(()).spawn();
    }
}
