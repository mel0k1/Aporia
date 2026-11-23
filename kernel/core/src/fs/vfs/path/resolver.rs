use alloc::str;

use ostd::task::Task;

use super::{Mount, MountTopology, Path, PerMountFlags};
use crate::{
    fs::{
        file::{
            InodeType, Permission,
            file_table::{FileDesc, RawFileDesc, get_file_fast},
        },
        utils::{PATH_MAX, SYMLINKS_MAX},
        vfs::{inode::SymbolicLink, path::MountNamespace},
    },
    prelude::*,
    process::{
        credentials::capabilities::CapSet, pid_table::PidTable, posix_thread::AsPosixThread,
    },
    security::lsm::hooks as lsm_hooks,
};

pub(crate) const AT_FDCWD: RawFileDesc = -100;

pub(crate) const AT_EMPTY_PATH: u32 = 0x1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmptyPathStr {
    Reject,

    AllowIfFlag(u32),

    Allow,
}

#[derive(Clone, Debug)]
pub(crate) struct PathResolver {
    root: Path,
    cwd: Path,
}

impl PathResolver {
    pub(super) fn new(root: Path, cwd: Path) -> Self {
        Self { root, cwd }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub(crate) fn chdir(&mut self, path: Path) -> Result<()> {
        path.check_dir_search_permission()?;

        self.set_cwd(path);
        Ok(())
    }

    pub(crate) fn chroot(&mut self, path: Path, ctx: &Context) -> Result<()> {
        path.check_dir_search_permission()?;

        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            ctx.thread_local.borrow_user_ns().as_ref(),
            ctx.posix_thread,
            CapSet::SYS_CHROOT,
        ))?;

        self.set_root(path);
        Ok(())
    }

    fn set_cwd(&mut self, path: Path) {
        self.cwd = path;
    }

    fn set_root(&mut self, path: Path) {
        self.root = path;
    }

    pub(crate) fn make_abs_path(&self, path: &Path) -> AbsPathResult {
        if path == &self.root {
            return AbsPathResult::Reachable("/".to_string());
        }
        if path.is_pseudo() {
            return AbsPathResult::Unreachable(path.name());
        }

        let mut components = VecDeque::new();
        components.push_front(self.resolve_name(path));

        let mut parent_path = self.resolve_parent(path);
        let mut reach_resolver_root = false;

        while let Some(parent_dir) = parent_path {
            if parent_dir == self.root {
                reach_resolver_root = true;
                break;
            }

            let parent_name = self.resolve_name(&parent_dir);
            if parent_name == "/" {
                break;
            }

            components.push_front(parent_name);

            parent_path = self.resolve_parent(&parent_dir);
        }

        let path_name = alloc::format!("/{}", components.make_contiguous().join("/"));
        debug_assert!(path_name.starts_with('/'));

        if reach_resolver_root {
            AbsPathResult::Reachable(path_name)
        } else {
            AbsPathResult::Unreachable(path_name)
        }
    }

    fn resolve_name(&self, path: &Path) -> String {
        let mut owned;
        let mut current = path;

        loop {
            if current == &self.root {
                return "/".to_string();
            }

            if !current.is_mount_root() {
                return current.name();
            }

            let Some(parent) = current
                .mount_node()
                .parent()
                .and_then(|parent| parent.upgrade())
            else {
                return current.name();
            };
            let Some(mountpoint) = current.mount_node().mountpoint() else {
                return current.name();
            };

            owned = Path::new(parent, mountpoint);
            current = &owned;
        }
    }

    fn resolve_parent(&self, path: &Path) -> Option<Path> {
        if !path.is_mount_root() && path.dentry.parent().is_none() {
            debug_assert!(path.is_pseudo());
            return None;
        }

        let mut owned;
        let mut current = path;

        loop {
            if current == &self.root {
                return None;
            }

            if !current.is_mount_root() {
                return Some(Path::new(
                    current.mount.clone(),
                    current.dentry.parent().unwrap(),
                ));
            }

            let parent = current.mount.parent()?;
            let mountpoint = current.mount.mountpoint()?;

            owned = Path::new(parent.upgrade()?, mountpoint);
            current = &owned;
        }
    }

    pub(crate) fn switch_to_mnt_ns(&mut self, mnt_ns: &Arc<MountNamespace>) -> Result<()> {
        if mnt_ns.owns(self.root.mount_node()) && mnt_ns.owns(self.cwd.mount_node()) {
            return Ok(());
        }

        let topology_guard = MountTopology::read_lock();

        let new_root = self
            .root
            .find_corresponding_mount(mnt_ns, &topology_guard)
            .ok_or_else(|| {
                errno!(
                    EINVAL,
                    "the root directory does not exist in the target mount namespace",
                )
            })?;
        let new_cwd = self
            .cwd
            .find_corresponding_mount(mnt_ns, &topology_guard)
            .ok_or_else(|| {
                errno!(
                    EINVAL,
                    "the current working directory does not exist in the target mount namespace",
                )
            })?;

        self.root = new_root;
        self.cwd = new_cwd;

        Ok(())
    }

    pub(in crate::fs) fn switch_root_for_boot(&mut self, new_root_mount: Arc<Mount>) {
        let mut topology_guard = MountTopology::write_lock();
        let parent_path = {
            let parent_mount = self.root.mount.parent().unwrap().upgrade().unwrap();
            let mountpoint = self.root.mount.mountpoint().unwrap();
            Path::new(parent_mount, mountpoint)
        };

        self.root.mount.detach_from_parent(&mut topology_guard);
        new_root_mount.graft_mount_tree(parent_path, &mut topology_guard);
        drop(topology_guard);

        let new_root = Path::new_root(new_root_mount);
        self.root = new_root.clone();
        self.cwd = new_root;
    }

    pub(crate) fn pivot_root(
        &mut self,
        new_root_path: FsPath,
        put_old_path: FsPath,
        pid_table: &PidTable,
        ctx: &Context,
    ) -> Result<()> {
        let new_root_path = self.lookup(&new_root_path)?;
        let put_old_path = self.lookup(&put_old_path)?;

        if new_root_path.type_() != InodeType::Dir || put_old_path.type_() != InodeType::Dir {
            return Err(errno!(
                ENOTDIR,
                "`new_root` or `put_old` is not a directory"
            ));
        }

        let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
        let current_mnt_ns = current_ns_proxy.mnt_ns();
        if !current_mnt_ns.owns(&new_root_path.mount) || !current_mnt_ns.owns(&put_old_path.mount) {
            return Err(errno!(
                EINVAL,
                "`new_root` or `put_old` is not in the current mount namespace"
            ));
        }

        if self.root.mount.id() == new_root_path.mount.id()
            || self.root.mount.id() == put_old_path.mount.id()
        {
            return Err(errno!(
                EBUSY,
                "`new_root` or `put_old` is on the current root mount"
            ));
        }
        if !new_root_path.is_mount_root() || !self.root.is_mount_root() {
            return Err(errno!(
                EINVAL,
                "`new_root` or the current root is not a mount point"
            ));
        }

        let mut topology_guard = MountTopology::write_lock();

        let Some(new_root_parent) = new_root_path
            .mount
            .parent()
            .and_then(|parent| parent.upgrade())
        else {
            return Err(errno!(EINVAL, "`new_root` is on the rootfs mount"));
        };
        let Some(current_root_parent) =
            self.root.mount.parent().and_then(|parent| parent.upgrade())
        else {
            return Err(errno!(EINVAL, "the current root is on the rootfs mount"));
        };

        if new_root_parent.is_shared()
            || current_root_parent.is_shared()
            || put_old_path.mount.is_shared()
        {
            return Err(errno!(
                EINVAL,
                "`new_root` parent, current root parent, or `put_old` mount is shared"
            ));
        }

        if !put_old_path.is_reachable_from(&new_root_path, &topology_guard) {
            return Err(errno!(
                EINVAL,
                "`put_old` is not at or underneath `new_root`"
            ));
        }
        if !new_root_path.is_reachable_from(&self.root, &topology_guard) {
            return Err(errno!(
                EINVAL,
                "`new_root` is not at or underneath the current root"
            ));
        }

        let parent_path = {
            let mountpoint = self.root.mount.mountpoint().unwrap();
            Path::new(current_root_parent, mountpoint)
        };

        self.root
            .mount
            .graft_mount_tree(put_old_path, &mut topology_guard);
        new_root_path
            .mount
            .graft_mount_tree(parent_path, &mut topology_guard);

        drop(topology_guard);

        for thread in pid_table.iter_threads() {
            let posix_thread = thread.as_posix_thread().unwrap();
            let ns_proxy = posix_thread.ns_proxy().lock();
            let Some(ns_proxy) = ns_proxy.as_ref() else {
                continue;
            };
            let mnt_ns = ns_proxy.mnt_ns();
            if !Arc::ptr_eq(mnt_ns, current_mnt_ns) {
                continue;
            }
            let fs = posix_thread.read_fs();
            let Some(fs) = fs.as_ref() else {
                continue;
            };
            if Arc::ptr_eq(fs, &ctx.thread_local.borrow_fs()) {
                continue;
            }

            let mut fs_resolver = fs.resolver().write();
            if fs_resolver.root() == &self.root {
                fs_resolver.set_root(new_root_path.clone());
            }
            if fs_resolver.cwd() == &self.root {
                fs_resolver.set_cwd(new_root_path.clone());
            }
        }

        if self.cwd == self.root {
            self.cwd = new_root_path.clone();
        }
        self.root = new_root_path;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum AbsPathResult {
    Reachable(String),
    Unreachable(String),
}

impl AbsPathResult {
    pub(crate) fn into_string(self) -> String {
        match self {
            AbsPathResult::Reachable(s) => s,
            AbsPathResult::Unreachable(s) => s,
        }
    }
}

impl PathResolver {
    pub(in crate::fs) fn collect_visible_mounts(&self) -> Vec<Arc<Mount>> {
        let _topology_guard = MountTopology::read_lock();
        let mut visible = Vec::new();
        let mut stack = vec![self.root.mount.clone()];
        let is_root_mount_root = self.root.is_mount_root();

        while let Some(mount) = stack.pop() {
            let is_root_mount = Arc::ptr_eq(&mount, &self.root.mount);

            if !is_root_mount || is_root_mount_root {
                visible.push(mount.clone());
            }

            let children: Vec<Arc<Mount>> = mount.children.read().values().cloned().collect();
            for child_mount in children {
                if is_root_mount && !is_root_mount_root {
                    let Some(mountpoint) = child_mount.mountpoint() else {
                        continue;
                    };
                    if !mountpoint.is_equal_or_descendant_of(&self.root.dentry) {
                        continue;
                    }
                }
                stack.push(child_mount);
            }
        }

        visible
    }
}

impl PathResolver {
    pub(crate) fn lookup_at_path(&self, path: &Path, name: &str) -> Result<Path> {
        let dir_dentry = path.dentry.as_dir_dentry_or_err()?;

        if path.inode().check_permission(Permission::MAY_EXEC).is_err() {
            return Err(errno!(EACCES, "the path cannot be looked up"));
        }
        let target_path = if super::is_dot(name) {
            return Ok(path.this());
        } else if super::is_dotdot(name) {
            self.resolve_parent(path).unwrap_or_else(|| path.this())
        } else {
            let target_dentry = dir_dentry.lookup_child(name)?;
            Path::new(path.mount.clone(), target_dentry)
        };

        Ok(target_path.get_top_path())
    }

    pub(crate) fn lookup(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_unresolved(fs_path)?.into_path()
    }

    pub(crate) fn lookup_no_follow(&self, fs_path: &FsPath) -> Result<Path> {
        self.lookup_unresolved_no_follow(fs_path)?.into_path()
    }

    pub(crate) fn lookup_unresolved(&self, fs_path: &FsPath) -> Result<LookupResult> {
        self.lookup_inner(fs_path, true)
    }

    pub(crate) fn lookup_unresolved_no_follow(&self, fs_path: &FsPath) -> Result<LookupResult> {
        self.lookup_inner(fs_path, false)
    }

    fn lookup_inner(&self, fs_path: &FsPath, follow_tail_link: bool) -> Result<LookupResult> {
        let lookup_res = match fs_path.inner {
            FsPathInner::Absolute(path) => {
                self.lookup_from_parent(&self.root, path.trim_start_matches('/'), follow_tail_link)?
            }
            FsPathInner::CwdRelative(path) => {
                self.lookup_from_parent(&self.cwd, path, follow_tail_link)?
            }
            FsPathInner::Cwd => LookupResult::Resolved(self.cwd.clone()),
            FsPathInner::FdRelative(fd, path) => {
                let parent = {
                    let task = Task::current().unwrap();
                    let mut file_table = task.as_thread_local().unwrap().borrow_file_table_mut();
                    let file = get_file_fast!(&mut file_table, fd);
                    file.path().clone()
                };
                self.lookup_from_parent(&parent, path, follow_tail_link)?
            }
            FsPathInner::Fd(fd) => {
                let task = Task::current().unwrap();
                let mut file_table = task.as_thread_local().unwrap().borrow_file_table_mut();
                let file = get_file_fast!(&mut file_table, fd);
                LookupResult::Resolved(file.path().clone())
            }
        };

        Ok(lookup_res)
    }

    #[expect(clippy::redundant_closure)]
    fn lookup_from_parent(
        &self,
        parent: &Path,
        relative_path: &str,
        follow_tail_link: bool,
    ) -> Result<LookupResult> {
        debug_assert!(!relative_path.starts_with('/'));

        if relative_path.len() > PATH_MAX {
            return Err(errno!(ENAMETOOLONG, "the path is too long"));
        }
        if relative_path.is_empty() {
            return Ok(LookupResult::Resolved(parent.clone()));
        }

        let mut link_path_opt = None;
        let mut follows = 0;

        let (mut current_path, mut relative_path) = (parent.clone(), relative_path);

        while !relative_path.is_empty() {
            let (next_name, path_remain, target_is_dir) =
                if let Some((prefix, suffix)) = relative_path.split_once('/') {
                    let suffix = suffix.trim_start_matches('/');
                    (prefix, suffix, true)
                } else {
                    (relative_path, "", false)
                };

            let next_is_tail = path_remain.is_empty();
            let next_path = match self.lookup_at_path(&current_path, next_name) {
                Ok(child) => child,
                Err(e) => {
                    if next_is_tail && e.error() == Errno::ENOENT {
                        return Ok(LookupResult::AtParent(LookupParentResult::new(
                            current_path,
                            next_name.to_string(),
                            target_is_dir,
                        )));
                    }
                    return Err(e);
                }
            };

            let next_type = next_path.type_();
            if next_type == InodeType::SymLink
                && (follow_tail_link || !next_is_tail || target_is_dir)
            {
                if next_path
                    .mount_node()
                    .flags()
                    .contains(PerMountFlags::NOSYMFOLLOW)
                {
                    return Err(errno!(
                        ELOOP,
                        "the symlink is on a file system mounted with `nosymfollow`"
                    ));
                }
                if follows >= SYMLINKS_MAX {
                    return Err(errno!(ELOOP, "there are too many symlinks"));
                }
                let read_link_res = next_path.inode().read_link()?;
                match read_link_res {
                    SymbolicLink::Plain(mut tmp_link_path) => {
                        let link_path_remain = {
                            if tmp_link_path.is_empty() {
                                return Err(errno!(ENOENT, "the symlink path is empty"));
                            }
                            if !next_is_tail {
                                tmp_link_path += "/";
                                tmp_link_path += path_remain;
                            } else if target_is_dir {
                                tmp_link_path += "/";
                            }
                            tmp_link_path
                        };

                        if link_path_remain.starts_with('/') {
                            current_path = self.root.clone();
                        }
                        let link_path = link_path_opt.get_or_insert_with(|| String::new());
                        link_path.clear();
                        link_path.push_str(link_path_remain.trim_start_matches('/'));
                        relative_path = link_path;
                        follows += 1;
                    }
                    SymbolicLink::Path(path) => {
                        current_path = path;
                        relative_path = path_remain;
                        follows += 1;
                    }
                }
            } else {
                if target_is_dir && next_type != InodeType::Dir {
                    return Err(errno!(ENOTDIR, "the inode is not a directory"));
                }
                current_path = next_path;
                relative_path = path_remain;
            }
        }

        Ok(LookupResult::Resolved(current_path))
    }
}

pub(crate) enum LookupResult {
    Resolved(Path),
    AtParent(LookupParentResult),
}

impl LookupResult {
    fn into_path(self) -> Result<Path> {
        match self {
            LookupResult::Resolved(target) => Ok(target),
            LookupResult::AtParent(_) => Err(errno!(
                ENOENT,
                "path resolution did not reach the final target",
            )),
        }
    }

    pub(crate) fn into_parent_and_filename(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return Err(errno!(EEXIST, "the path already exists"));
        };
        res.into_parent_and_filename()
    }

    pub(crate) fn into_parent_and_basename(self) -> Result<(Path, String)> {
        let LookupResult::AtParent(res) = self else {
            return Err(errno!(EEXIST, "the path already exists"));
        };
        Ok(res.into_parent_and_basename())
    }
}

pub(crate) struct LookupParentResult {
    parent: Path,
    unresolved_name: String,
    target_is_dir: bool,
}

impl LookupParentResult {
    fn new(parent: Path, unresolved_name: String, target_is_dir: bool) -> Self {
        Self {
            parent,
            unresolved_name,
            target_is_dir,
        }
    }

    pub(crate) fn target_is_dir(&self) -> bool {
        self.target_is_dir
    }

    pub(crate) fn into_parent_and_filename(self) -> Result<(Path, String)> {
        if self.target_is_dir {
            return Err(errno!(ENOENT, "the path is a directory"));
        }
        Ok((self.parent, self.unresolved_name))
    }

    pub(crate) fn into_parent_and_basename(self) -> (Path, String) {
        (self.parent, self.unresolved_name)
    }
}

#[derive(Debug)]
pub(crate) struct FsPath<'a> {
    inner: FsPathInner<'a>,
}

#[derive(Debug)]
enum FsPathInner<'a> {
    Absolute(&'a str),
    CwdRelative(&'a str),
    Cwd,
    FdRelative(FileDesc, &'a str),
    Fd(FileDesc),
}

impl<'a> FsPath<'a> {
    pub(crate) fn from_fd(dirfd: RawFileDesc) -> Result<Self> {
        let fs_path_inner = if dirfd == AT_FDCWD {
            FsPathInner::Cwd
        } else {
            FsPathInner::Fd(FileDesc::try_from(dirfd)?)
        };

        Ok(Self {
            inner: fs_path_inner,
        })
    }

    pub(crate) fn from_fd_at(
        dirfd: RawFileDesc,
        path: &'a str,
        policy: EmptyPathStr,
    ) -> Result<Self> {
        if path.is_empty() {
            let allowed = match policy {
                EmptyPathStr::Reject => false,
                EmptyPathStr::AllowIfFlag(f) => f & AT_EMPTY_PATH != 0,
                EmptyPathStr::Allow => dirfd != AT_FDCWD,
            };
            if !allowed {
                return Err(errno!(ENOENT, "the path is empty"));
            }
            return Self::from_fd(dirfd);
        }

        if path.len() > PATH_MAX {
            return Err(errno!(ENAMETOOLONG, "the path is too long"));
        }

        let fs_path_inner = if path.starts_with('/') {
            FsPathInner::Absolute(path)
        } else if dirfd == AT_FDCWD {
            FsPathInner::CwdRelative(path)
        } else {
            FsPathInner::FdRelative(FileDesc::try_from(dirfd)?, path)
        };

        Ok(Self {
            inner: fs_path_inner,
        })
    }
}

impl<'a> TryFrom<&'a str> for FsPath<'a> {
    type Error = Error;

    fn try_from(path: &'a str) -> Result<FsPath<'a>> {
        FsPath::from_fd_at(AT_FDCWD, path, EmptyPathStr::Reject)
    }
}

pub(crate) trait SplitPath {
    fn split_dirname_and_basename(&self) -> core::result::Result<(&Self, &Self), SplitPathError>;
}

impl SplitPath for str {
    fn split_dirname_and_basename(&self) -> core::result::Result<(&Self, &Self), SplitPathError> {
        if self.is_empty() {
            return Err(SplitPathError::Empty);
        }

        let trimmed = self.trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(SplitPathError::Root);
        }

        if let Some(pos) = trimmed.rfind('/') {
            let dirname = trimmed[..pos].trim_end_matches('/');
            Ok((
                if dirname.is_empty() { "/" } else { dirname },
                &trimmed[pos + 1..],
            ))
        } else {
            Ok((".", trimmed))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SplitPathError {
    Empty,
    Root,
}

impl SplitPathError {
    pub(crate) fn reject_root_as_busy(self) -> Error {
        match self {
            Self::Empty => errno!(ENOENT, "the path is empty"),
            Self::Root => {
                errno!(EBUSY, "the path refers to the root directory")
            }
        }
    }

    pub(crate) fn reject_root_as_is_dir(self) -> Error {
        match self {
            Self::Empty => errno!(ENOENT, "the path is empty"),
            Self::Root => {
                errno!(EISDIR, "the path refers to the root directory")
            }
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    type SplitResult = Result<(&'static str, &'static str), SplitPathError>;

    #[track_caller]
    fn assert_split_results(
        cases: &Vec<(&'static str, SplitResult)>,
        split: impl Fn(&str) -> Result<(&str, &str), SplitPathError>,
    ) {
        for case in cases.iter() {
            let result = split(case.0);
            assert_eq!(result, case.1, "splitting '{}' failed", case.0);
        }
    }

    #[ktest]
    fn path_split_basename() {
        let cases = vec![
            ("", Err(SplitPathError::Empty)),
            ("/", Err(SplitPathError::Root)),
            ("///", Err(SplitPathError::Root)),
            ("///.", Ok(("/", "."))),
            ("//./", Ok(("/", "."))),
            ("a", Ok((".", "a"))),
            ("/a", Ok(("/", "a"))),
            ("b/a", Ok(("b", "a"))),
            ("/b/a", Ok(("/b", "a"))),
            ("a", Ok((".", "a"))),
            ("//a", Ok(("/", "a"))),
            ("b//a", Ok(("b", "a"))),
            ("//b//a", Ok(("//b", "a"))),
            ("a/", Ok((".", "a"))),
            ("/a/", Ok(("/", "a"))),
            ("b/a/", Ok(("b", "a"))),
            ("/b/a/", Ok(("/b", "a"))),
            ("a//", Ok((".", "a"))),
            ("/a//", Ok(("/", "a"))),
            ("b/a//", Ok(("b", "a"))),
            ("/b/a//", Ok(("/b", "a"))),
            (" a ", Ok((".", " a "))),
            (" //a ", Ok((" ", "a "))),
            (" b//a ", Ok((" b", "a "))),
            (" //b//a ", Ok((" //b", "a "))),
            (" a/ ", Ok((" a", " "))),
            (" //a/ ", Ok((" //a", " "))),
            (" b//a/ ", Ok((" b//a", " "))),
            (" //b//a/ ", Ok((" //b//a", " "))),
        ];
        assert_split_results(&cases, SplitPath::split_dirname_and_basename);
    }

    #[ktest]
    fn fs_path_from_fd_at_policy_matrix() {
        assert!(
            FsPath::from_fd_at(AT_FDCWD, "", EmptyPathStr::Reject)
                .is_err_and(|e| e.error() == Errno::ENOENT)
        );
        assert!(
            FsPath::from_fd_at(3, "", EmptyPathStr::Reject)
                .is_err_and(|e| e.error() == Errno::ENOENT)
        );

        assert!(
            FsPath::from_fd_at(3, "", EmptyPathStr::AllowIfFlag(0))
                .is_err_and(|e| e.error() == Errno::ENOENT)
        );
        assert!(FsPath::from_fd_at(3, "", EmptyPathStr::AllowIfFlag(AT_EMPTY_PATH)).is_ok());
        assert!(
            FsPath::from_fd_at(3, "", EmptyPathStr::AllowIfFlag(0x100))
                .is_err_and(|e| e.error() == Errno::ENOENT)
        );

        assert!(
            FsPath::from_fd_at(AT_FDCWD, "", EmptyPathStr::Allow)
                .is_err_and(|e| e.error() == Errno::ENOENT)
        );
        assert!(FsPath::from_fd_at(3, "", EmptyPathStr::Allow).is_ok());
    }

    #[ktest]
    fn fs_path_from_fd_at_rejects_too_long() {
        let long = "a".repeat(PATH_MAX + 1);
        assert!(
            FsPath::from_fd_at(AT_FDCWD, &long, EmptyPathStr::Reject)
                .is_err_and(|e| e.error() == Errno::ENAMETOOLONG)
        );
    }
}
