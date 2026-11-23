use ostd::{
    sync::Waker,
    task::{CurrentTask, Task},
};

use crate::prelude::*;

pub(crate) struct TaskSet {
    tasks: Vec<Arc<Task>>,
    has_exited_main: bool,
    has_exited_group: bool,
    in_execve: bool,
    execve_waker: Option<Arc<Waker>>,
}

impl TaskSet {
    pub(super) fn new() -> Self {
        Self {
            tasks: Vec::new(),
            has_exited_main: false,
            has_exited_group: false,
            in_execve: false,
            execve_waker: None,
        }
    }

    pub(super) fn insert(&mut self, task: Arc<Task>) -> Result<(), Arc<Task>> {
        if self.has_exited_group || self.in_execve {
            return Err(task);
        }

        self.tasks.push(task);
        Ok(())
    }

    pub(super) fn remove_exited(&mut self, task: &CurrentTask) -> bool {
        let position = self
            .tasks
            .iter()
            .position(|some_task| core::ptr::eq(some_task.as_ref(), task.as_ref()))
            .unwrap();

        if position == 0 {
            assert!(!self.has_exited_main);
            self.has_exited_main = true;
        } else {
            self.tasks.swap_remove(position);
        }

        if let Some(waker) = self.execve_waker.as_ref() {
            waker.wake_up();
        }

        self.has_exited_main && self.tasks.len() == 1
    }

    pub(super) fn has_exited_main(&self) -> bool {
        self.has_exited_main
    }

    pub(super) fn swap_main(&mut self) {
        self.tasks.swap_remove(0);
        self.has_exited_main = false;
    }

    pub(super) fn set_exited_group(&mut self) {
        debug_assert!(!self.in_execve);
        self.has_exited_group = true;
    }

    pub(super) fn has_exited_group(&self) -> bool {
        self.has_exited_group
    }

    pub(super) fn start_execve(&mut self) {
        debug_assert!(!self.has_exited_group);
        self.in_execve = true;
    }

    pub(super) fn finish_execve(&mut self) {
        self.in_execve = false;
    }

    pub(super) fn in_execve(&self) -> bool {
        self.in_execve
    }

    pub(super) fn set_execve_waker(&mut self, waker: Arc<Waker>) {
        debug_assert!(self.execve_waker.is_none());
        self.execve_waker = Some(waker);
    }

    pub(super) fn clear_execve_waker(&mut self) {
        self.execve_waker = None;
    }
}

impl TaskSet {
    pub(crate) fn as_slice(&self) -> &[Arc<Task>] {
        self.tasks.as_slice()
    }

    pub(crate) fn main(&self) -> &Arc<Task> {
        &self.tasks[0]
    }
}
