mod heap;
mod init_stack;

use core::ops::Range;
#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::task::disable_preempt;

pub(crate) use self::{
    heap::{Heap, LockedHeap},
    init_stack::{
        INIT_STACK_SIZE, InitStack, InitStackReader, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
        aux_vec::{AuxKey, AuxVec},
    },
};
use crate::{
    fs::vfs::path::Path,
    prelude::*,
    vm::vmar::{Vmar, VmarHandle},
};

pub(crate) struct ProcessVm {
    init_stack: InitStack,
    heap: Heap,
    code_range: SpinLock<Range<Vaddr>>,
    data_range: SpinLock<Range<Vaddr>>,
    executable_path: Path,
    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    vdso_base: AtomicUsize,
}

impl ProcessVm {
    pub(crate) fn new(executable_path: Path) -> Self {
        Self {
            init_stack: InitStack::new(),
            heap: Heap::new_uninitialized(),
            code_range: SpinLock::new(0..0),
            data_range: SpinLock::new(0..0),
            executable_path,
            #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
            vdso_base: AtomicUsize::new(0),
        }
    }

    pub(crate) fn fork_from(process_vm: &Self, heap_guard: &LockedHeap) -> Self {
        Self {
            init_stack: process_vm.init_stack.clone(),
            heap: Heap::fork_from(heap_guard),
            code_range: SpinLock::new(process_vm.code_range.lock().clone()),
            data_range: SpinLock::new(process_vm.data_range.lock().clone()),
            executable_path: process_vm.executable_path.clone(),
            #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
            vdso_base: AtomicUsize::new(process_vm.vdso_base.load(Ordering::Relaxed)),
        }
    }

    pub(crate) fn init_stack(&self) -> &InitStack {
        &self.init_stack
    }

    pub(crate) fn heap(&self) -> &Heap {
        &self.heap
    }

    pub(crate) fn code_range(&self) -> Range<Vaddr> {
        self.code_range.lock().clone()
    }

    pub(crate) fn data_range(&self) -> Range<Vaddr> {
        self.data_range.lock().clone()
    }

    pub(crate) fn executable_path(&self) -> &Path {
        &self.executable_path
    }

    pub(super) fn map_and_write_init_stack(
        &self,
        vmar: &Vmar,
        argv: Vec<CString>,
        envp: Vec<CString>,
        aux_vec: AuxVec,
    ) -> Result<()> {
        self.init_stack().map_and_write(vmar, argv, envp, aux_vec)
    }

    pub(super) fn map_and_init_heap(
        &self,
        vmar: &Vmar,
        data_segment_size: usize,
        heap_base: Vaddr,
    ) -> Result<()> {
        self.heap()
            .map_and_init_heap(vmar, data_segment_size, heap_base)
    }

    pub(super) fn set_code_range(&self, range: Range<Vaddr>) {
        *self.code_range.lock() = range;
    }

    pub(super) fn set_data_range(&self, range: Range<Vaddr>) {
        *self.data_range.lock() = range;
    }

    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    pub(super) fn vdso_base(&self) -> Vaddr {
        self.vdso_base.load(Ordering::Relaxed)
    }

    #[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
    pub(super) fn set_vdso_base(&self, addr: Vaddr) {
        self.vdso_base.store(addr, Ordering::Relaxed);
    }
}

pub(crate) struct ProcessVmarGuard<'a> {
    inner: MutexGuard<'a, Option<Arc<Vmar>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct VmarSnapshot(Weak<Vmar>);

impl VmarSnapshot {
    pub(crate) fn as_ptr(&self) -> *const Vmar {
        Weak::as_ptr(&self.0)
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl From<Weak<Vmar>> for VmarSnapshot {
    fn from(snapshot: Weak<Vmar>) -> Self {
        Self(snapshot)
    }
}

impl<'a> ProcessVmarGuard<'a> {
    pub(super) fn new(inner: MutexGuard<'a, Option<Arc<Vmar>>>) -> Self {
        Self { inner }
    }

    pub(crate) fn unwrap(&self) -> &Vmar {
        self.inner.as_ref().unwrap()
    }

    pub(crate) fn as_ref(&self) -> Option<&Vmar> {
        self.inner.as_ref().map(|v| &**v)
    }

    pub(crate) fn snapshot(&self) -> VmarSnapshot {
        VmarSnapshot(self.inner.as_ref().map(Arc::downgrade).unwrap_or_default())
    }

    pub(crate) fn is_same_as(&self, snapshot: &VmarSnapshot) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|vmar| core::ptr::eq(Arc::as_ptr(vmar), Weak::as_ptr(&snapshot.0)))
    }

    pub(super) fn set_vmar(&mut self, new_vmar: Option<Arc<Vmar>>) -> Option<Arc<Vmar>> {
        core::mem::replace(&mut *self.inner, new_vmar)
    }

    pub(crate) fn init_stack_reader(&self) -> Option<InitStackReader<'_>> {
        self.as_ref()
            .map(|vmar| vmar.process_vm().init_stack.reader(vmar))
    }
}

pub(super) fn activate_vmar<'a>(
    ctx: &'a Context<'a>,
    new_vmar: VmarHandle,
) -> (ProcessVmarGuard<'a>, VmarHandle) {
    let vmar_arc = new_vmar.clone_arc();

    let mut vmar_guard = ctx.process.lock_vmar();

    let old_vmar = {
        let _preempt_guard = disable_preempt();
        let old_vmar = ctx
            .thread_local
            .vmar()
            .borrow_mut()
            .replace(new_vmar)
            .unwrap();
        vmar_arc.vm_space().activate();
        old_vmar
    };
    vmar_guard.set_vmar(Some(vmar_arc));

    (vmar_guard, old_vmar)
}
