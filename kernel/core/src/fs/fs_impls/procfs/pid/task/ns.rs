use core::marker::PhantomData;

use crate::{
    fs::{
        cgroupfs::CgroupNamespace,
        file::{InodeType, mkmod},
        procfs::{
            pid::TidDirOps,
            template::{
                ListedEntry, ProcDir, ProcDirOps, ProcSym, ProcSymOps, ReaddirEntry,
                visit_listed_entries,
            },
        },
        pseudofs::NsCommonOps,
        vfs::{
            inode::{Inode, RevalidationPolicy, SymbolicLink},
            path::{MountNamespace, Path},
        },
    },
    ipc::IpcNamespace,
    net::uts_ns::UtsNamespace,
    prelude::*,
    process::{NsProxy, UserNamespace, posix_thread::AsPosixThread},
    thread::Thread,
};

pub(super) struct NsDirOps {
    dir: TidDirOps,
}

impl NsDirOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self { dir: dir.clone() }, parent, mkmod!(u+r, a+x))
    }
}

#[derive(Clone, Copy)]
enum NsProxyEntry {
    Cgroup,
    Ipc,
    Mnt,
    Uts,
}

impl NsProxyEntry {
    const ALL: &[Self] = &[Self::Cgroup, Self::Ipc, Self::Mnt, Self::Uts];

    fn as_str(self) -> &'static str {
        match self {
            Self::Cgroup => "cgroup",
            Self::Ipc => "ipc",
            Self::Mnt => "mnt",
            Self::Uts => "uts",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "cgroup" => Some(Self::Cgroup),
            "ipc" => Some(Self::Ipc),
            "mnt" => Some(Self::Mnt),
            "uts" => Some(Self::Uts),
            _ => None,
        }
    }

    fn new_sym_inode(
        self,
        dir: &TidDirOps,
        ns_proxy: &NsProxy,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        match self {
            Self::Cgroup => NsSymOps::<CgroupNamespace>::new_inode(
                dir.clone(),
                ns_proxy.cgroup_ns().get_path(),
                parent,
            ),
            Self::Ipc => NsSymOps::<IpcNamespace>::new_inode(
                dir.clone(),
                ns_proxy.ipc_ns().get_path(),
                parent,
            ),
            Self::Mnt => NsSymOps::<MountNamespace>::new_inode(
                dir.clone(),
                ns_proxy.mnt_ns().get_path(),
                parent,
            ),
            Self::Uts => NsSymOps::<UtsNamespace>::new_inode(
                dir.clone(),
                ns_proxy.uts_ns().get_path(),
                parent,
            ),
        }
    }
}

fn cached_ns_path(inode: &dyn Inode) -> Option<&Path> {
    if let Some(sym) = inode.downcast_ref::<NsSymlink<CgroupNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<IpcNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<MountNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UserNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    if let Some(sym) = inode.downcast_ref::<NsSymlink<UtsNamespace>>() {
        return Some(&sym.inner().ns_path);
    }
    None
}

impl ProcDirOps for NsDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.dir.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if name == "user" {
            let Some(process) = self.dir.process() else {
                return Err(errno!(ESRCH, "the process does not exist"));
            };

            let user_ns = process.user_ns().lock();
            return Ok(NsSymOps::<UserNamespace>::new_inode(
                self.dir.clone(),
                user_ns.get_path(),
                this_dir.this_weak().clone(),
            ));
        }

        let entry = NsProxyEntry::from_str(name)
            .ok_or_else(|| errno!(ENOENT, "the file does not exist"))?;

        let Some(thread) = self.dir.thread() else {
            return Err(errno!(ESRCH, "the thread does not exist"));
        };

        let ns_proxy_guard = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let ns_proxy = ns_proxy_guard
            .as_ref()
            .ok_or_else(|| errno!(ENOENT, "the thread has exited"))?;
        Ok(entry.new_sym_inode(&self.dir, ns_proxy, this_dir.this_weak().clone()))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        let Some(thread) = self.dir.thread() else {
            return Err(errno!(ENOENT, "the thread does not exist"));
        };

        let has_ns_proxy = thread
            .as_posix_thread()
            .unwrap()
            .ns_proxy()
            .lock()
            .as_ref()
            .is_some();
        let ns_proxy_entries = has_ns_proxy
            .then_some(NsProxyEntry::ALL)
            .into_iter()
            .flat_map(|entries| {
                entries
                    .iter()
                    .map(|entry| ListedEntry::new(entry.as_str(), InodeType::SymLink))
            });

        let user_entry = Some(ListedEntry::new("user", InodeType::SymLink)).into_iter();

        visit_listed_entries(offset, ns_proxy_entries.chain(user_entry), visit_fn)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, child: &dyn Inode) -> bool {
        let Some(cached_path) = cached_ns_path(child) else {
            return false;
        };

        if child.downcast_ref::<NsSymlink<UserNamespace>>().is_some() {
            let Some(process) = self.dir.process() else {
                return false;
            };
            let user_ns = process.user_ns().lock();
            return cached_path == &user_ns.get_path();
        }

        let Some(thread) = self.dir.thread() else {
            return false;
        };
        let ns_proxy = thread.as_posix_thread().unwrap().ns_proxy().lock();
        let Some(ns_proxy) = ns_proxy.as_ref() else {
            return false;
        };

        if child.downcast_ref::<NsSymlink<CgroupNamespace>>().is_some() {
            return cached_path == &ns_proxy.cgroup_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<MountNamespace>>().is_some() {
            return cached_path == &ns_proxy.mnt_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<UtsNamespace>>().is_some() {
            return cached_path == &ns_proxy.uts_ns().get_path();
        }

        if child.downcast_ref::<NsSymlink<IpcNamespace>>().is_some() {
            return cached_path == &ns_proxy.ipc_ns().get_path();
        }

        false
    }
}

type NsSymlink<T> = ProcSym<NsSymOps<T>>;

struct NsSymOps<T: NsCommonOps> {
    dir: TidDirOps,
    ns_path: Path,
    phantom: PhantomData<T>,
}

impl<T: NsCommonOps> NsSymOps<T> {
    fn new_inode(dir: TidDirOps, ns_path: Path, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(
            Self {
                dir,
                ns_path,
                phantom: PhantomData,
            },
            parent,
            mkmod!(a+rwx),
        )
    }
}

impl<T: NsCommonOps> ProcSymOps for NsSymOps<T> {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.dir.thread()
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Path(self.ns_path.clone()))
    }
}
