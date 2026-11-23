use core::cell::{Cell, Ref, RefCell, RefMut};

#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::{FsBase, GsBase};
use ostd::{
    arch::cpu::context::FpuContext, irq::DisabledLocalIrqGuard, sync::RwArc, task::CurrentTask,
};

use super::{RobustListHead, cpu_sync::CpuSync};
use crate::{
    fs::{file::file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        NsProxy, UserNamespace,
        signal::{SigStack, sig_mask::SigMask},
    },
    vm::vmar::VmarHandle,
};

pub(crate) struct ThreadLocal {
    set_child_tid: Cell<Vaddr>,
    clear_child_tid: Cell<Vaddr>,

    vmar: RefCell<Option<VmarHandle>>,
    page_fault_disabled: Cell<bool>,

    robust_list: RefCell<Option<RobustListHead>>,

    file_table: RefCell<Option<RwArc<FileTable>>>,
    fs: RefCell<Option<Arc<ThreadFsInfo>>>,

    supp_user_context: SuppUserContext,

    sig_stack: RefCell<SigStack>,
    sig_mask_saved: Cell<Option<SigMask>>,
    orig_syscall_ret: Cell<Option<usize>>,

    user_ns: RefCell<Arc<UserNamespace>>,
    ns_proxy: RefCell<Option<Arc<NsProxy>>>,
}

impl ThreadLocal {
    #[expect(clippy::too_many_arguments)]
    pub(super) fn new(
        set_child_tid: Vaddr,
        clear_child_tid: Vaddr,
        vmar: VmarHandle,
        file_table: RwArc<FileTable>,
        fs: Arc<ThreadFsInfo>,
        supp_user_context: SuppUserContext,
        user_ns: Arc<UserNamespace>,
        ns_proxy: Arc<NsProxy>,
    ) -> Self {
        Self {
            set_child_tid: Cell::new(set_child_tid),
            clear_child_tid: Cell::new(clear_child_tid),
            vmar: RefCell::new(Some(vmar)),
            page_fault_disabled: Cell::new(false),
            robust_list: RefCell::new(None),
            file_table: RefCell::new(Some(file_table)),
            fs: RefCell::new(Some(fs)),
            supp_user_context,
            sig_stack: RefCell::new(SigStack::default()),
            sig_mask_saved: Cell::new(None),
            orig_syscall_ret: Cell::new(None),
            user_ns: RefCell::new(user_ns),
            ns_proxy: RefCell::new(Some(ns_proxy)),
        }
    }

    pub(crate) fn set_child_tid(&self) -> &Cell<Vaddr> {
        &self.set_child_tid
    }

    pub(crate) fn clear_child_tid(&self) -> &Cell<Vaddr> {
        &self.clear_child_tid
    }

    pub(crate) fn vmar(&self) -> &RefCell<Option<VmarHandle>> {
        &self.vmar
    }

    pub(crate) fn with_page_fault_disabled<F, T>(&self, func: F) -> Option<Result<T>>
    where
        F: FnOnce() -> Result<T>,
    {
        let is_disabled = self.is_page_fault_disabled();
        self.page_fault_disabled.set(true);

        let result = func();

        self.page_fault_disabled.set(is_disabled);

        if result
            .as_ref()
            .is_err_and(|err| err.error() == Errno::EFAULT)
        {
            None
        } else {
            Some(result)
        }
    }

    pub(crate) fn is_page_fault_disabled(&self) -> bool {
        self.page_fault_disabled.get()
    }

    pub(crate) fn robust_list(&self) -> &RefCell<Option<RobustListHead>> {
        &self.robust_list
    }

    pub(crate) fn borrow_file_table(&self) -> FileTableRef<'_> {
        Ref::map(self.file_table.borrow(), |file_table| {
            file_table.as_ref().unwrap()
        })
    }

    pub(crate) fn borrow_file_table_mut(&self) -> FileTableRefMut<'_> {
        RefMut::map(self.file_table.borrow_mut(), |file_table| {
            file_table.as_mut().unwrap()
        })
    }

    pub(super) fn remove_file_table(&self) -> Option<RwArc<FileTable>> {
        self.file_table.borrow_mut().take()
    }

    pub(crate) fn borrow_fs(&self) -> Ref<'_, Arc<ThreadFsInfo>> {
        Ref::map(self.fs.borrow(), |fs| fs.as_ref().unwrap())
    }

    pub(in crate::process) fn borrow_fs_mut(&self) -> RefMut<'_, Arc<ThreadFsInfo>> {
        RefMut::map(self.fs.borrow_mut(), |fs| fs.as_mut().unwrap())
    }

    pub(super) fn remove_fs(&self) -> Option<Arc<ThreadFsInfo>> {
        self.fs.borrow_mut().take()
    }

    pub(crate) fn is_fs_shared(&self) -> bool {
        Arc::strong_count(&self.borrow_fs()) > 2
    }

    pub(crate) fn supp_user_context(&self) -> &SuppUserContext {
        &self.supp_user_context
    }

    pub(crate) fn sig_stack(&self) -> &RefCell<SigStack> {
        &self.sig_stack
    }

    pub(in crate::process) fn sig_mask_saved(&self) -> &Cell<Option<SigMask>> {
        &self.sig_mask_saved
    }

    pub(in crate::process) fn orig_syscall_ret(&self) -> Option<usize> {
        self.orig_syscall_ret.get()
    }

    pub(crate) fn set_orig_syscall_ret(&self, value: Option<usize>) {
        self.orig_syscall_ret.set(value);
    }

    pub(crate) fn borrow_user_ns(&self) -> Ref<'_, Arc<UserNamespace>> {
        self.user_ns.borrow()
    }

    pub(crate) fn borrow_ns_proxy(&self) -> Ref<'_, Arc<NsProxy>> {
        Ref::map(self.ns_proxy.borrow(), |ns_proxy| {
            ns_proxy.as_ref().unwrap()
        })
    }

    pub(in crate::process) fn borrow_ns_proxy_mut(&self) -> RefMut<'_, Arc<NsProxy>> {
        RefMut::map(self.ns_proxy.borrow_mut(), |ns_proxy| {
            ns_proxy.as_mut().unwrap()
        })
    }

    pub(super) fn remove_ns_proxy(&self) -> Option<Arc<NsProxy>> {
        self.ns_proxy.borrow_mut().take()
    }
}

pub(crate) struct SuppUserContext {
    fpu: CpuSync<FpuContext>,
    #[cfg(target_arch = "x86_64")]
    fs_base: CpuSync<FsBase>,
    #[cfg(target_arch = "x86_64")]
    gs_base: CpuSync<GsBase>,
}

impl SuppUserContext {
    pub(crate) fn new() -> Self {
        Self {
            fpu: CpuSync::new(FpuContext::new()),
            #[cfg(target_arch = "x86_64")]
            fs_base: CpuSync::new(FsBase::default()),
            #[cfg(target_arch = "x86_64")]
            gs_base: CpuSync::new(GsBase::default()),
        }
    }

    pub(crate) fn with_fpu_context(mut self, ctx: FpuContext) -> Self {
        self.fpu = CpuSync::new(ctx);
        self
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn with_fs_base(mut self, fs_base: FsBase) -> Self {
        self.fs_base = CpuSync::new(fs_base);
        self
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn with_gs_base(mut self, gs_base: GsBase) -> Self {
        self.gs_base = CpuSync::new(gs_base);
        self
    }

    pub(crate) fn fpu(&self) -> &CpuSync<FpuContext> {
        &self.fpu
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn fs_base(&self) -> &CpuSync<FsBase> {
        &self.fs_base
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn gs_base(&self) -> &CpuSync<GsBase> {
        &self.gs_base
    }

    pub(crate) fn before_schedule(&self, guard: &DisabledLocalIrqGuard) {
        self.fpu.before_schedule(guard);
        #[cfg(target_arch = "x86_64")]
        {
            self.fs_base.before_schedule(guard);
            self.gs_base.before_schedule(guard);
        }
    }

    pub(crate) fn before_user_exec(&self, guard: &DisabledLocalIrqGuard) {
        self.fpu.before_user_exec(guard);
        #[cfg(target_arch = "x86_64")]
        {
            self.fs_base.before_user_exec(guard);
            self.gs_base.before_user_exec(guard);
        }
    }
}

pub(crate) type FileTableRef<'a> = Ref<'a, RwArc<FileTable>>;

pub(crate) type FileTableRefMut<'a> = RefMut<'a, RwArc<FileTable>>;

pub(crate) trait AsThreadLocal {
    fn as_thread_local(&self) -> Option<&ThreadLocal>;
}

impl AsThreadLocal for CurrentTask {
    fn as_thread_local(&self) -> Option<&ThreadLocal> {
        self.local_data().downcast_ref()
    }
}
