use spin::Once;

use crate::{
    fs::{cgroupfs::CgroupNamespace, vfs::path::MountNamespace},
    ipc::IpcNamespace,
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{CloneFlags, Process, UserNamespace, posix_thread::PosixThread},
};

pub(crate) struct NsProxy {
    cgroup_ns: Arc<CgroupNamespace>,
    ipc_ns: Arc<IpcNamespace>,
    mnt_ns: Arc<MountNamespace>,
    uts_ns: Arc<UtsNamespace>,
}

impl NsProxy {
    pub(in crate::process) fn get_init_singleton() -> &'static Arc<Self> {
        static INIT: Once<Arc<NsProxy>> = Once::new();
        INIT.call_once(|| {
            Arc::new(NsProxy {
                cgroup_ns: CgroupNamespace::get_init_singleton().clone(),
                ipc_ns: IpcNamespace::get_init_singleton().clone(),
                mnt_ns: MountNamespace::get_init_singleton().clone(),
                uts_ns: UtsNamespace::get_init_singleton().clone(),
            })
        })
    }

    pub(in crate::process) fn new_clone(
        self: &Arc<Self>,
        user_ns: &Arc<UserNamespace>,
        process: &Process,
        posix_thread: &PosixThread,
        clone_flags: CloneFlags,
    ) -> Result<Arc<Self>> {
        let clone_ns_flags = (clone_flags & CloneFlags::CLONE_NS_FLAGS) - CloneFlags::CLONE_NEWUSER;

        if clone_ns_flags.is_empty() {
            return Ok(self.clone());
        }

        check_unsupported_ns_flags(clone_ns_flags)?;

        let mut builder = NsProxyBuilder::new(self);

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWCGROUP) {
            let current_cgroup = process.cgroup().get().as_deref().map(Arc::clone);
            let new_cgroup_ns =
                CgroupNamespace::new_clone(current_cgroup, user_ns.clone(), posix_thread)?;
            builder.cgroup_ns(new_cgroup_ns);
        }

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWIPC) {
            let new_ipc_ns = self.ipc_ns.new_clone(user_ns.clone(), posix_thread)?;
            builder.ipc_ns(new_ipc_ns);
        }

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWNS) {
            let new_mnt_ns = self.mnt_ns.new_clone(user_ns.clone(), posix_thread)?;
            builder.mnt_ns(new_mnt_ns);
        }

        if clone_ns_flags.contains(CloneFlags::CLONE_NEWUTS) {
            let new_uts_ns = self.uts_ns.new_clone(user_ns.clone(), posix_thread)?;
            builder.uts_ns(new_uts_ns);
        }

        Ok(Arc::new(builder.build()))
    }

    pub(crate) fn cgroup_ns(&self) -> &Arc<CgroupNamespace> {
        &self.cgroup_ns
    }

    pub(crate) fn ipc_ns(&self) -> &Arc<IpcNamespace> {
        &self.ipc_ns
    }

    pub(crate) fn mnt_ns(&self) -> &Arc<MountNamespace> {
        &self.mnt_ns
    }

    pub(crate) fn uts_ns(&self) -> &Arc<UtsNamespace> {
        &self.uts_ns
    }
}

pub(crate) struct NsProxyBuilder<'a> {
    old_proxy: &'a NsProxy,

    cgroup_ns: Option<Arc<CgroupNamespace>>,
    ipc_ns: Option<Arc<IpcNamespace>>,
    mnt_ns: Option<Arc<MountNamespace>>,
    uts_ns: Option<Arc<UtsNamespace>>,
}

impl<'a> NsProxyBuilder<'a> {
    pub(crate) fn new(old_proxy: &'a NsProxy) -> Self {
        Self {
            old_proxy,
            cgroup_ns: None,
            ipc_ns: None,
            mnt_ns: None,
            uts_ns: None,
        }
    }

    pub(crate) fn cgroup_ns(&mut self, cgroup_ns: Arc<CgroupNamespace>) -> &mut Self {
        self.cgroup_ns = Some(cgroup_ns);
        self
    }

    pub(crate) fn ipc_ns(&mut self, ipc_ns: Arc<IpcNamespace>) -> &mut Self {
        self.ipc_ns = Some(ipc_ns);
        self
    }

    pub(crate) fn mnt_ns(&mut self, mnt_ns: Arc<MountNamespace>) -> &mut Self {
        self.mnt_ns = Some(mnt_ns);
        self
    }

    pub(crate) fn uts_ns(&mut self, uts_ns: Arc<UtsNamespace>) -> &mut Self {
        self.uts_ns = Some(uts_ns);
        self
    }

    pub(crate) fn build(self) -> NsProxy {
        let Self {
            old_proxy,
            cgroup_ns: new_cgroup,
            ipc_ns: new_ipc,
            mnt_ns: new_mnt,
            uts_ns: new_uts,
        } = self;

        let new_cgroup = new_cgroup.unwrap_or_else(|| old_proxy.cgroup_ns.clone());
        let new_ipc = new_ipc.unwrap_or_else(|| old_proxy.ipc_ns.clone());
        let new_mnt = new_mnt.unwrap_or_else(|| old_proxy.mnt_ns.clone());
        let new_uts = new_uts.unwrap_or_else(|| old_proxy.uts_ns.clone());

        NsProxy {
            cgroup_ns: new_cgroup,
            ipc_ns: new_ipc,
            mnt_ns: new_mnt,
            uts_ns: new_uts,
        }
    }
}

pub(crate) fn check_unsupported_ns_flags(flags: CloneFlags) -> Result<()> {
    const SUPPORTED_FLAGS: CloneFlags = CloneFlags::CLONE_NEWCGROUP
        .union(CloneFlags::CLONE_NEWIPC)
        .union(CloneFlags::CLONE_NEWNS)
        .union(CloneFlags::CLONE_NEWUTS);

    let unsupported_flags =
        (flags & CloneFlags::CLONE_NS_FLAGS) - SUPPORTED_FLAGS - CloneFlags::CLONE_NEWUSER;
    if unsupported_flags.is_empty() {
        return Ok(());
    }

    warn!("unsupported clone ns flags: {:?}", unsupported_flags);
    Err(errno!(EINVAL, "unsupported clone namespace flags"))
}

pub(crate) trait ContextSetNsAdminApi {
    fn set_ns_proxy(&self, ns_proxy: Arc<NsProxy>);
}

impl ContextSetNsAdminApi for Context<'_> {
    fn set_ns_proxy(&self, ns_proxy: Arc<NsProxy>) {
        let mut pthread_ns_proxy = self.posix_thread.ns_proxy().lock();
        let mut thread_local_ns_proxy = self.thread_local.borrow_ns_proxy_mut();

        if !Arc::ptr_eq(&thread_local_ns_proxy.mnt_ns, &ns_proxy.mnt_ns) {
            *self.thread_local.borrow_fs().resolver().write() = ns_proxy.mnt_ns.new_path_resolver();
        }

        *pthread_ns_proxy = Some(ns_proxy.clone());
        *thread_local_ns_proxy = ns_proxy;
    }
}
