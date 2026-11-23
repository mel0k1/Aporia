use core::{
    ops::Deref,
    sync::atomic::{AtomicU32, Ordering},
};

use hashbrown::HashMap;
use ostd::sync::{RwMutexUpgradeableGuard, RwMutexWriteGuard};

use super::{RenameMode, is_dot, is_dot_or_dotdot, is_dotdot};
use crate::{
    fs::{
        self,
        file::{InodeMode, InodeType, Permission},
        vfs::{
            inode::{Inode, MknodType, RevalidationPolicy},
            inode_ext::InodeExt,
        },
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

pub(crate) struct Dentry {
    inode: Arc<dyn Inode>,
    type_: InodeType,
    name_and_parent: NameAndParent,
    dir_state: Option<DentryDirState>,
    flags: AtomicU32,
    mount_count: AtomicU32,
    this: Weak<Dentry>,
}

struct DentryDirState {
    children: RwMutex<DentryChildren>,
    revalidation_policy: RevalidationPolicy,
}

enum NameAndParent {
    Real(Option<RwLock<(String, Arc<Dentry>)>>),
    Anonymous(Arc<Dentry>),
    Pseudo(fn(&dyn Inode) -> String),
}

#[derive(Debug)]
struct SetNameAndParentError;

impl NameAndParent {
    fn name(&self, inode: &dyn Inode) -> String {
        match self {
            NameAndParent::Real(name_and_parent) => match name_and_parent {
                Some(name_and_parent) => name_and_parent.read().0.clone(),
                None => String::from("/"),
            },
            NameAndParent::Anonymous(_) => String::from("/"),
            NameAndParent::Pseudo(name_fn) => (name_fn)(inode),
        }
    }

    fn parent(&self) -> Option<Arc<Dentry>> {
        match self {
            NameAndParent::Real(Some(name_and_parent)) => Some(name_and_parent.read().1.clone()),
            NameAndParent::Anonymous(parent) => Some(parent.clone()),
            NameAndParent::Real(None) | NameAndParent::Pseudo(_) => None,
        }
    }

    fn set(&self, name: &str, parent: Arc<Dentry>) -> Result<(), SetNameAndParentError> {
        if let NameAndParent::Real(Some(name_and_parent)) = self {
            let mut name_and_parent = name_and_parent.write();
            *name_and_parent = (String::from(name), parent);
            Ok(())
        } else {
            Err(SetNameAndParentError)
        }
    }
}

impl Dentry {
    pub(in crate::fs) fn new_root(inode: Arc<dyn Inode>) -> Arc<Self> {
        Self::new(inode, DentryOptions::Root)
    }

    pub(super) fn new_anonymous(inode: Arc<dyn Inode>, parent: &DirDentry) -> Arc<Self> {
        Self::new(
            inode,
            DentryOptions::Anonymous {
                parent: parent.this(),
            },
        )
    }

    pub(super) fn new_pseudo(
        inode: Arc<dyn Inode>,
        name_fn: fn(&dyn Inode) -> String,
    ) -> Arc<Self> {
        Self::new(inode, DentryOptions::Pseudo(name_fn))
    }

    fn new(inode: Arc<dyn Inode>, options: DentryOptions) -> Arc<Self> {
        let name_and_parent = match options {
            DentryOptions::Root => NameAndParent::Real(None),
            DentryOptions::Named(name_and_parent) => {
                NameAndParent::Real(Some(RwLock::new(name_and_parent)))
            }
            DentryOptions::Anonymous { parent } => NameAndParent::Anonymous(parent),
            DentryOptions::Pseudo(name_fn) => NameAndParent::Pseudo(name_fn),
        };

        let type_ = inode.type_();
        let is_dir = type_ == InodeType::Dir;
        let dir_state = is_dir.then(|| DentryDirState {
            children: RwMutex::new(DentryChildren::new()),
            revalidation_policy: inode.revalidation_policy(),
        });

        Arc::new_cyclic(|weak_self| Self {
            inode,
            type_,
            name_and_parent,
            dir_state,
            flags: AtomicU32::new(DentryFlags::empty().bits()),
            mount_count: AtomicU32::new(0),
            this: weak_self.clone(),
        })
    }

    pub(super) fn is_pseudo(&self) -> bool {
        matches!(self.name_and_parent, NameAndParent::Pseudo(_))
    }

    pub(in crate::fs) fn type_(&self) -> InodeType {
        self.type_
    }

    pub(in crate::fs) fn name(&self) -> String {
        self.name_and_parent.name(self.inode.as_ref())
    }

    pub(in crate::fs) fn parent(&self) -> Option<Arc<Self>> {
        self.name_and_parent.parent()
    }

    fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }

    pub(super) fn key(&self) -> DentryKey {
        DentryKey::new(self)
    }

    pub(in crate::fs) fn inode(&self) -> &Arc<dyn Inode> {
        &self.inode
    }

    fn flags(&self) -> DentryFlags {
        let flags = self.flags.load(Ordering::Relaxed);
        DentryFlags::from_bits(flags).unwrap()
    }

    pub(in crate::fs) fn is_equal_or_descendant_of(&self, ancestor: &Arc<Self>) -> bool {
        let mut current = Some(self.this());

        while let Some(node) = current {
            if Arc::ptr_eq(&node, ancestor) {
                return true;
            }
            current = node.parent();
        }

        false
    }

    pub(super) fn is_mountpoint(&self) -> bool {
        self.flags().contains(DentryFlags::MOUNTED)
    }

    pub(super) fn inc_mount_count(&self) {
        let old_count = self.mount_count.fetch_add(1, Ordering::Relaxed);
        if old_count == 0 {
            self.flags
                .fetch_or(DentryFlags::MOUNTED.bits(), Ordering::Relaxed);
        }
    }

    pub(super) fn dec_mount_count(&self) {
        let old_count = self.mount_count.fetch_sub(1, Ordering::Relaxed);
        if old_count == 1 {
            self.flags
                .fetch_and(!(DentryFlags::MOUNTED.bits()), Ordering::Relaxed);
        }
    }

    pub(in crate::fs) fn path_name(&self) -> String {
        let mut path_name = self.name().to_string();
        let mut current_dir = self.this();

        while let Some(parent_dir) = current_dir.parent() {
            path_name = {
                let parent_name = parent_dir.name();
                if parent_name != "/" {
                    parent_name + "/" + &path_name
                } else {
                    parent_name + &path_name
                }
            };
            current_dir = parent_dir;
        }

        debug_assert!(path_name.starts_with('/') || self.is_pseudo());
        path_name
    }

    pub(in crate::fs) fn as_dir_dentry_or_err(&self) -> Result<DirDentry<'_>> {
        debug_assert_eq!(self.dir_state.is_some(), self.type_ == InodeType::Dir);

        let Some(dir_state) = &self.dir_state else {
            return Err(errno!(
                ENOTDIR,
                "the dentry is not related to a directory inode"
            ));
        };

        Ok(DirDentry {
            inner: self,
            children: &dir_state.children,
            revalidation_policy: dir_state.revalidation_policy,
        })
    }
}

pub(in crate::fs) struct DirDentry<'a> {
    inner: &'a Dentry,
    children: &'a RwMutex<DentryChildren>,
    revalidation_policy: RevalidationPolicy,
}

impl Deref for DirDentry<'_> {
    type Target = Dentry;

    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl DirDentry<'_> {
    pub(in crate::fs) fn create_child(
        &self,
        name: &str,
        create_inode_fn: impl FnOnce() -> Result<Arc<dyn Inode>>,
    ) -> Result<Arc<Dentry>> {
        let children = self.validate_child_absent(name)?;
        let new_inode = create_inode_fn()?;
        let mut children = children.upgrade();
        let new_child = Dentry::new(
            new_inode,
            DentryOptions::Named((String::from(name), self.this())),
        );

        Ok(self.insert_positive_child(&mut children, name, new_child))
    }

    fn validate_name_len(&self, name: &str) -> Result<()> {
        let max_namelen = self.inode.fs().max_name_len();
        if name.len() > max_namelen {
            return Err(errno!(ENAMETOOLONG, "the path name is too long"));
        }
        Ok(())
    }

    fn validate_child_absent<'a>(
        &'a self,
        name: &str,
    ) -> Result<RwMutexUpgradeableGuard<'a, DentryChildren>> {
        self.validate_name_len(name)?;

        let mut children = self.children.upread();

        if let Some(cached_entry) = children.find(name) {
            if self.revalidate_cached_entry(name, &cached_entry) {
                return match cached_entry {
                    CachedDentry::Positive { .. } => {
                        return Err(errno!(EEXIST, "the dentry already exists"));
                    }
                    CachedDentry::Negative => Ok(children),
                };
            }

            let mut children_for_update = children.upgrade();
            let _ = children_for_update.remove(name);
            children = children_for_update.downgrade();
        }

        match self.inode.lookup(name) {
            Ok(inode) => {
                let existing_child = Dentry::new(
                    inode,
                    DentryOptions::Named((String::from(name), self.this())),
                );
                let mut children_for_update = children.upgrade();
                self.insert_positive_child(&mut children_for_update, name, existing_child);
                Err(errno!(EEXIST, "the dentry already exists"))
            }
            Err(error) if error.error() == Errno::ENOENT => Ok(children),
            Err(error) => Err(error),
        }
    }

    fn insert_positive_child(
        &self,
        children: &mut DentryChildren,
        name: &str,
        child: Arc<Dentry>,
    ) -> Arc<Dentry> {
        children.insert_positive(String::from(name), child.clone());
        self.revalidate_positive_children_if_needed(children);

        child
    }

    fn has_sticky_bit(&self) -> Result<bool> {
        Ok(self.inode.metadata()?.mode.has_sticky_bit())
    }

    fn check_sticky_bit_permission(&self, child_inode: &Arc<dyn Inode>) -> Result<()> {
        let current_thread = current_thread!();
        let Some(posix_thread) = current_thread.as_posix_thread() else {
            return Ok(());
        };

        let dir_metadata = self.inode.metadata()?;
        let child_metadata = child_inode.metadata()?;
        let fsuid = posix_thread.credentials().fsuid();
        if fsuid == dir_metadata.uid || fsuid == child_metadata.uid {
            return Ok(());
        }

        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            posix_thread,
            CapSet::FOWNER,
        ))
    }

    fn resolve_child_for_rename(
        &self,
        children: &mut DentryChildren,
        name: &str,
    ) -> Result<Arc<Dentry>> {
        if let Some(cached_entry) = children.find(name) {
            if !self.revalidate_cached_entry(name, &cached_entry) {
                let _ = children.remove(name);
            } else if cached_entry.is_mountpoint() {
                return Err(errno!(EBUSY, "dentry is mountpoint"));
            } else {
                return match cached_entry {
                    CachedDentry::Positive { dentry } => Ok(dentry),
                    CachedDentry::Negative => {
                        return Err(errno!(ENOENT, "found a negative dentry"));
                    }
                };
            }
        }

        self.inode.lookup(name).map(|inode| {
            let dentry = Dentry::new(
                inode,
                DentryOptions::Named((String::from(name), self.this())),
            );
            self.insert_positive_child(children, name, dentry)
        })
    }

    fn revalidate_positive_children_if_needed(&self, children: &mut DentryChildren) {
        const POSITIVE_CHILD_REVALIDATION_INTERVAL: usize = 1024;
        const POSITIVE_CHILD_REVALIDATION_CACHE_SIZE_THRESHOLD: usize = 1024;

        if !self
            .revalidation_policy
            .contains(RevalidationPolicy::REVALIDATE_EXISTS)
        {
            return;
        }

        children.insert_count = children.insert_count.wrapping_add(1);
        if !children
            .insert_count
            .is_multiple_of(POSITIVE_CHILD_REVALIDATION_INTERVAL)
            || children.entries.len() <= POSITIVE_CHILD_REVALIDATION_CACHE_SIZE_THRESHOLD
        {
            return;
        }

        children.revalidate_positive_entries(self);
    }

    pub(in crate::fs) fn lookup_child(&self, name: &str) -> Result<Arc<Dentry>> {
        debug_assert!(!is_dot_or_dotdot(name));
        self.validate_name_len(name)?;

        let mut children = self.children.upread();

        if let Some(cached_entry) = children.find(name) {
            if self.revalidate_cached_entry(name, &cached_entry) {
                return match cached_entry {
                    CachedDentry::Positive { dentry } => Ok(dentry),
                    CachedDentry::Negative => {
                        return Err(errno!(ENOENT, "found a negative dentry"));
                    }
                };
            }

            let mut children_for_update = children.upgrade();
            let _ = children_for_update.remove(name);
            children = children_for_update.downgrade();
        }

        let inode = match self.inode.lookup(name) {
            Ok(inode) => inode,
            Err(error) if error.error() == Errno::ENOENT => {
                children.upgrade().insert_negative(name.to_string());
                return Err(error);
            }
            Err(error) => return Err(error),
        };

        let target = Dentry::new(
            inode,
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();

        Ok(self.insert_positive_child(&mut children, name, target))
    }

    pub(in crate::fs) fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<Dentry>> {
        let children = self.validate_child_absent(name)?;
        Self::check_mknod_capability(&type_)?;
        let inode = self.inode.mknod(self, name, mode, type_)?;
        let new_child = Dentry::new(
            inode,
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();

        Ok(self.insert_positive_child(&mut children, name, new_child))
    }

    fn check_mknod_capability(type_: &MknodType) -> Result<()> {
        if !matches!(type_, MknodType::CharDevice(_) | MknodType::BlockDevice(_)) {
            return Ok(());
        }

        let current_thread = current_thread!();
        let Some(posix_thread) = current_thread.as_posix_thread() else {
            return Ok(());
        };

        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            posix_thread,
            CapSet::MKNOD,
        ))
    }

    pub(in crate::fs) fn link(&self, old_dentry: &Dentry, name: &str) -> Result<()> {
        let children = self.validate_child_absent(name)?;
        self.inode.link(self, old_dentry, name)?;
        let dentry = Dentry::new(
            old_dentry.inode().clone(),
            DentryOptions::Named((String::from(name), self.this())),
        );
        let mut children = children.upgrade();
        self.insert_positive_child(&mut children, name, dentry.clone());
        fs::vfs::notify::on_link(dentry.parent().unwrap().inode(), dentry.inode(), || {
            name.to_string()
        });
        Ok(())
    }

    pub(in crate::fs) fn unlink(&self, name: &str) -> Result<()> {
        if is_dot_or_dotdot(name) {
            return Err(errno!(EISDIR, "unlink on . or .."));
        }

        let dir_inode = self.inode();
        let child_dentry = self.remove_child(name, |dir_inode, child| dir_inode.unlink(child))?;
        let child_inode = child_dentry.inode();

        let nlinks = child_inode.metadata()?.nr_hard_links;
        fs::vfs::notify::on_link_count(child_inode);
        if nlinks == 0 {
            fs::vfs::notify::on_inode_removed(child_inode);
        }
        fs::vfs::notify::on_delete(dir_inode, child_inode, || name.to_string());
        if nlinks == 0 {
            let publisher = child_inode.fs_event_publisher_or_init();
            let removed_nr_subscribers = publisher.disable_new_and_remove_subscribers();
            child_inode
                .fs()
                .fs_event_subscriber_stats()
                .remove_subscribers(removed_nr_subscribers);
        }
        Ok(())
    }

    pub(in crate::fs) fn rmdir(&self, name: &str) -> Result<()> {
        if is_dot(name) {
            return Err(errno!(EINVAL, "rmdir on ."));
        }
        if is_dotdot(name) {
            return Err(errno!(ENOTEMPTY, "rmdir on .."));
        }

        let dir_inode = self.inode();
        let child_dentry = self.remove_child(name, |dir_inode, child| dir_inode.rmdir(child))?;
        let child_inode = child_dentry.inode();

        let nlinks = child_inode.metadata()?.nr_hard_links;
        if nlinks == 0 {
            fs::vfs::notify::on_inode_removed(child_inode);
        }
        fs::vfs::notify::on_delete(dir_inode, child_inode, || name.to_string());
        if nlinks == 0 {
            let publisher = child_inode.fs_event_publisher_or_init();
            let removed_nr_subscribers = publisher.disable_new_and_remove_subscribers();
            child_inode
                .fs()
                .fs_event_subscriber_stats()
                .remove_subscribers(removed_nr_subscribers);
        }
        Ok(())
    }

    fn remove_child(
        &self,
        name: &str,
        remove_child_fn: impl FnOnce(&dyn Inode, &Dentry) -> Result<()>,
    ) -> Result<Arc<Dentry>> {
        self.validate_name_len(name)?;

        let dir_inode = self.inode();
        let mut children = self.children.upread();
        let cached_child = match children.find(name) {
            None => None,
            Some(cached_entry) if !self.revalidate_cached_entry(name, &cached_entry) => {
                let mut children_for_update = children.upgrade();
                let _ = children_for_update.remove(name);
                children = children_for_update.downgrade();
                None
            }
            Some(cached_entry) => {
                if cached_entry.is_mountpoint() {
                    return Err(errno!(EBUSY, "dentry is mountpoint"));
                }

                match cached_entry {
                    CachedDentry::Positive { dentry } => Some(dentry),
                    CachedDentry::Negative => {
                        return Err(errno!(ENOENT, "found a negative dentry"));
                    }
                }
            }
        };

        let child_dentry = match &cached_child {
            Some(child) => child.clone(),
            None => Dentry::new(
                dir_inode.lookup(name)?,
                DentryOptions::Named((String::from(name), self.this())),
            ),
        };

        if self.has_sticky_bit()? {
            self.check_sticky_bit_permission(child_dentry.inode())?;
        }

        remove_child_fn(dir_inode.as_ref(), &child_dentry)?;
        if cached_child.is_some() {
            children.upgrade().delete(name);
        }
        Ok(child_dentry)
    }

    pub(in crate::fs) fn rename(
        &self,
        old_name: &str,
        new_dir: &DirDentry,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        if is_dot_or_dotdot(old_name) {
            return Err(errno!(EBUSY, "old_name is . or .."));
        }
        if is_dot_or_dotdot(new_name) {
            if mode == RenameMode::NoReplace {
                return Err(errno!(EEXIST, "new_name is . or .."));
            } else {
                return Err(errno!(EBUSY, "new_name is . or .."));
            }
        }

        self.validate_name_len(old_name)?;
        new_dir.validate_name_len(new_name)?;

        let old_dir_inode = self.inode();

        if core::ptr::eq(self.inner, new_dir.inner) {
            if old_name == new_name {
                match mode {
                    RenameMode::Replace | RenameMode::Exchange => return Ok(()),
                    RenameMode::NoReplace => {
                        return Err(errno!(EEXIST, "the new path already exists"));
                    }
                }
            }

            let mut children = self.children.write();

            let old_dentry = self.resolve_child_for_rename(&mut children, old_name)?;
            let old_inode = old_dentry.inode();
            let new_dentry = match self.resolve_child_for_rename(&mut children, new_name) {
                Ok(new_dentry) => {
                    if Arc::ptr_eq(old_inode, new_dentry.inode()) {
                        return Ok(());
                    }
                    Some(new_dentry)
                }
                Err(e) if e.error() == Errno::ENOENT => None,
                Err(e) => return Err(e),
            };

            Self::check_rename_mode(mode, new_dentry.as_ref())?;

            let replaced_inode = new_dentry.as_ref().map(|dentry| dentry.inode());

            if self.has_sticky_bit()? {
                self.check_sticky_bit_permission(old_inode)?;
                if let Some(replaced_inode) = replaced_inode {
                    self.check_sticky_bit_permission(replaced_inode)?;
                }
            }

            old_dir_inode.rename(&old_dentry, self, new_name, replaced_inode, mode)?;

            match mode {
                RenameMode::Replace | RenameMode::NoReplace => {
                    children.delete(old_name);
                    old_dentry
                        .name_and_parent
                        .set(new_name, self.this())
                        .unwrap();
                    self.insert_positive_child(&mut children, new_name, old_dentry);
                }
                RenameMode::Exchange => {
                    let new_dentry = new_dentry.unwrap();
                    old_dentry
                        .name_and_parent
                        .set(new_name, self.this())
                        .unwrap();
                    new_dentry
                        .name_and_parent
                        .set(old_name, self.this())
                        .unwrap();
                    self.insert_positive_child(&mut children, new_name, old_dentry);
                    self.insert_positive_child(&mut children, old_name, new_dentry);
                }
            }
        } else {
            let (mut old_children, mut new_children) =
                write_lock_children_on_two_dentries(self, new_dir);

            let old_dentry = self.resolve_child_for_rename(&mut old_children, old_name)?;
            let old_inode = old_dentry.inode();
            let new_dentry = match new_dir.resolve_child_for_rename(&mut new_children, new_name) {
                Ok(new_dentry) => {
                    if Arc::ptr_eq(old_inode, new_dentry.inode()) {
                        return Ok(());
                    }
                    Some(new_dentry)
                }
                Err(e) if e.error() == Errno::ENOENT => None,
                Err(e) => return Err(e),
            };

            Self::check_rename_mode(mode, new_dentry.as_ref())?;
            Self::check_rename_cycle(mode, self, &old_dentry, new_dir, new_dentry.as_ref())?;

            let replaced_inode = new_dentry.as_ref().map(|dentry| dentry.inode());

            if self.has_sticky_bit()? {
                self.check_sticky_bit_permission(old_inode)?;
            }
            if new_dir.has_sticky_bit()?
                && let Some(replaced_inode) = replaced_inode
            {
                new_dir.check_sticky_bit_permission(replaced_inode)?;
            }

            if old_inode.type_() == InodeType::Dir {
                old_inode.check_permission(Permission::MAY_WRITE)?;
            }

            if mode == RenameMode::Exchange
                && let Some(replaced_inode) = &replaced_inode
                && replaced_inode.type_() == InodeType::Dir
            {
                replaced_inode.check_permission(Permission::MAY_WRITE)?;
            }

            old_dir_inode.rename(&old_dentry, new_dir, new_name, replaced_inode, mode)?;

            match mode {
                RenameMode::Replace | RenameMode::NoReplace => {
                    old_children.delete(old_name);
                    old_dentry
                        .name_and_parent
                        .set(new_name, new_dir.this())
                        .unwrap();
                    new_dir.insert_positive_child(&mut new_children, new_name, old_dentry);
                }
                RenameMode::Exchange => {
                    let new_dentry = new_dentry.unwrap();
                    old_dentry
                        .name_and_parent
                        .set(new_name, new_dir.this())
                        .unwrap();
                    new_dentry
                        .name_and_parent
                        .set(old_name, self.this())
                        .unwrap();
                    new_dir.insert_positive_child(&mut new_children, new_name, old_dentry);
                    self.insert_positive_child(&mut old_children, old_name, new_dentry);
                }
            }
        }
        Ok(())
    }

    fn check_rename_cycle(
        mode: RenameMode,
        old_dir: &DirDentry<'_>,
        old_dentry: &Arc<Dentry>,
        new_dir: &DirDentry<'_>,
        new_dentry: Option<&Arc<Dentry>>,
    ) -> Result<()> {
        if old_dentry.type_() == InodeType::Dir && new_dir.is_equal_or_descendant_of(old_dentry) {
            return Err(errno!(EINVAL, "the new path is inside the old directory"));
        }

        if mode == RenameMode::Exchange
            && let Some(new_dentry) = new_dentry
            && new_dentry.type_() == InodeType::Dir
            && old_dir.is_equal_or_descendant_of(new_dentry)
        {
            return Err(errno!(EINVAL, "the old path is inside the new directory"));
        }

        Ok(())
    }

    fn check_rename_mode(mode: RenameMode, new_dentry: Option<&Arc<Dentry>>) -> Result<()> {
        match mode {
            RenameMode::NoReplace if new_dentry.is_some() => {
                Err(errno!(EEXIST, "the new path already exists"))
            }
            RenameMode::Exchange if new_dentry.is_none() => {
                Err(errno!(ENOENT, "the new path does not exist"))
            }
            _ => Ok(()),
        }
    }

    fn revalidate_cached_entry(&self, name: &str, cached_entry: &CachedDentry) -> bool {
        let policy = self.revalidation_policy;
        if policy.is_empty() {
            return true;
        }

        if policy.contains(RevalidationPolicy::REVALIDATE_EXISTS)
            && let CachedDentry::Positive { dentry } = cached_entry
        {
            return self.inode.revalidate_exists(name, dentry.inode().as_ref());
        }

        if policy.contains(RevalidationPolicy::REVALIDATE_ABSENT)
            && let CachedDentry::Negative = cached_entry
        {
            return self.inode.revalidate_absent(name);
        }

        true
    }
}

impl Debug for Dentry {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let mut debug = f.debug_struct("Dentry");
        debug
            .field("inode", &self.inode)
            .field("type_", &self.type_)
            .field("flags", &self.flags)
            .field("mount_count", &self.mount_count);
        if let Some(dir_state) = &self.dir_state {
            debug
                .field("children", &*dir_state.children.read())
                .field("revalidation_policy", &dir_state.revalidation_policy);
        }
        debug.finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct DentryKey {
    name: String,
    parent_ptr: usize,
}

impl DentryKey {
    fn new(dentry: &Dentry) -> Self {
        debug_assert!(!matches!(
            dentry.name_and_parent,
            NameAndParent::Anonymous(_)
        ));

        let name = dentry.name();
        let parent = dentry.parent().unwrap_or_else(|| dentry.this());

        Self {
            name,
            parent_ptr: Arc::as_ptr(&parent) as usize,
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct DentryFlags: u32 {
        const MOUNTED = 1 << 0;
    }
}

enum DentryOptions {
    Root,
    Named((String, Arc<Dentry>)),
    Anonymous { parent: Arc<Dentry> },
    Pseudo(fn(&dyn Inode) -> String),
}

#[derive(Debug)]
struct DentryChildren {
    entries: HashMap<String, CachedDentry>,
    insert_count: usize,
}

#[derive(Clone, Debug)]
enum CachedDentry {
    Positive { dentry: Arc<Dentry> },
    Negative,
}

impl CachedDentry {
    fn new_positive(dentry: Arc<Dentry>) -> Self {
        Self::Positive { dentry }
    }

    fn new_negative() -> Self {
        Self::Negative
    }

    fn into_dentry(self) -> Option<Arc<Dentry>> {
        match self {
            Self::Positive { dentry } => Some(dentry),
            Self::Negative => None,
        }
    }

    fn is_mountpoint(&self) -> bool {
        match self {
            Self::Positive { dentry } => dentry.is_mountpoint(),
            Self::Negative => false,
        }
    }

    fn is_positive(&self) -> bool {
        matches!(self, Self::Positive { .. })
    }
}

#[cfg(debug_assertions)]
const NEGATIVE_ENTRY_LIMIT: usize = 10000;
#[cfg(debug_assertions)]
static NEGATIVE_ENTRY_COUNTER: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

impl DentryChildren {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            insert_count: 0,
        }
    }

    fn find(&self, name: &str) -> Option<CachedDentry> {
        self.entries.get(name).cloned()
    }

    fn insert_positive(&mut self, name: String, dentry: Arc<Dentry>) {
        let _prev = self
            .entries
            .insert(name, CachedDentry::new_positive(dentry));

        #[cfg(debug_assertions)]
        if matches!(_prev, Some(CachedDentry::Negative)) {
            NEGATIVE_ENTRY_COUNTER.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn insert_negative(&mut self, name: String) {
        let prev = self.entries.insert(name, CachedDentry::new_negative());
        debug_assert!(
            prev.is_none(),
            "insert_negative overwrote an existing entry",
        );

        #[cfg(debug_assertions)]
        {
            let new_count = NEGATIVE_ENTRY_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
            if new_count > NEGATIVE_ENTRY_LIMIT as u64 {
                warn!("number of negative dentries has reached {}", new_count);
            }
        }
    }

    fn delete(&mut self, name: &str) {
        let Some((_, exist_entry)) = self.entries.get_key_value_mut(name) else {
            return;
        };
        if exist_entry.is_positive() {
            #[cfg(debug_assertions)]
            {
                let new_count = NEGATIVE_ENTRY_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
                if new_count > NEGATIVE_ENTRY_LIMIT as u64 {
                    warn!("number of negative dentries has reached {}", new_count);
                }
            }
            *exist_entry = CachedDentry::new_negative()
        }
    }

    fn remove(&mut self, name: &str) -> Option<Arc<Dentry>> {
        let removed_entry = self.entries.remove(name);
        #[cfg(debug_assertions)]
        if matches!(removed_entry, Some(CachedDentry::Negative)) {
            NEGATIVE_ENTRY_COUNTER.fetch_sub(1, Ordering::Relaxed);
        }

        removed_entry.and_then(CachedDentry::into_dentry)
    }

    fn revalidate_positive_entries(&mut self, dir: &DirDentry<'_>) {
        self.entries
            .retain(|name, cached_entry| match cached_entry {
                CachedDentry::Positive { dentry } => {
                    dir.inode.revalidate_exists(name, dentry.inode().as_ref())
                }
                CachedDentry::Negative => true,
            });
    }
}

#[cfg(debug_assertions)]
impl Drop for DentryChildren {
    fn drop(&mut self) {
        let negative_count = self
            .entries
            .values()
            .filter(|entry| matches!(entry, CachedDentry::Negative))
            .count();

        NEGATIVE_ENTRY_COUNTER.fetch_sub(negative_count as u64, Ordering::Relaxed);
    }
}

fn write_lock_children_on_two_dentries<'a>(
    this: &'a DirDentry,
    other: &'a DirDentry,
) -> (
    RwMutexWriteGuard<'a, DentryChildren>,
    RwMutexWriteGuard<'a, DentryChildren>,
) {
    let this_key = this.key();
    let other_key = other.key();
    match this_key.cmp(&other_key) {
        core::cmp::Ordering::Less => {
            let this = this.children.write();
            let other = other.children.write();
            (this, other)
        }
        core::cmp::Ordering::Greater => {
            let other = other.children.write();
            let this = this.children.write();
            (this, other)
        }
        core::cmp::Ordering::Equal => {
            unreachable!("two distinct DirDentry's with identical DentryKey")
        }
    }
}
