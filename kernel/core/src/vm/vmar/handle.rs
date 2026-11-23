use alloc::sync::{Arc, Weak};
use core::ops::Deref;

use crate::{process::ProcessVm, vm::vmar::Vmar};

pub(crate) struct VmarHandle(Arc<Vmar>);

impl Deref for VmarHandle {
    type Target = Vmar;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for VmarHandle {
    fn drop(&mut self) {
        self.0.dec_num_handles();
    }
}

impl VmarHandle {
    pub(crate) fn new(process_vm: ProcessVm) -> Self {
        Self(Vmar::new(process_vm))
    }

    pub(crate) fn clone_handle(&self) -> Self {
        self.0.inc_num_handles();
        Self(self.0.clone())
    }

    pub(crate) fn clone_arc(&self) -> Arc<Vmar> {
        self.0.clone()
    }

    pub(crate) fn clone_weak(&self) -> Weak<Vmar> {
        Arc::downgrade(&self.0)
    }
}
