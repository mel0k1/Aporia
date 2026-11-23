use alloc::vec::Vec;

use ostd::sync::Mutex;
use spin::Once;

use crate::common::{DevNum, Error, Result, SysStr};

pub fn install_hooks(hooks: &'static dyn KernelHooks) {
    HOOKS.install(hooks);
}

pub trait KernelHooks: Send + Sync + 'static {
    fn create_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError>;

    fn delete_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError>;

    fn broadcast_uevent(&self, payload: Vec<u8>);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevNodeRequest {
    pub devnum: DevNum,
    pub path: SysStr,
    pub mode: Option<u16>,
}

#[derive(Clone, Copy, Debug)]
pub struct HookError;

pub(crate) fn create_devnode(request: DevNodeRequest) -> Result<()> {
    HOOKS.create_devnode(request)
}

pub(crate) fn delete_devnode(request: &DevNodeRequest) -> Result<()> {
    HOOKS.delete_devnode(request)
}

pub(crate) fn broadcast_uevent(payload: Vec<u8>) {
    if let Some(hooks) = HOOKS.hooks.get() {
        hooks.broadcast_uevent(payload);
    }
}

pub(crate) struct HookSlot {
    hooks: Once<&'static dyn KernelHooks>,
    pending: Mutex<Vec<DevNodeRequest>>,
}

impl HookSlot {
    pub(crate) const fn new() -> Self {
        Self {
            hooks: Once::new(),
            pending: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn install(&self, hooks: &'static dyn KernelHooks) {
        let mut pending = self.pending.lock();
        let installed = self.hooks.call_once(|| hooks);
        for request in core::mem::take(&mut *pending) {
            let _ = installed.create_devnode(&request);
        }
    }

    pub(crate) fn create_devnode(&self, request: DevNodeRequest) -> Result<()> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => {
                drop(queue);
                hooks.create_devnode(&request).map_err(|_| Error::Hook)
            }
            None => {
                queue.push(request);
                Ok(())
            }
        }
    }

    pub(crate) fn delete_devnode(&self, request: &DevNodeRequest) -> Result<()> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => {
                drop(queue);
                hooks.delete_devnode(request).map_err(|_| Error::Hook)
            }
            None => {
                queue.retain(|pending| pending != request);
                Ok(())
            }
        }
    }
}

static HOOKS: HookSlot = HookSlot::new();
