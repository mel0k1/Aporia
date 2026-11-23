#![short_vis_path::add(procfs)]

use alloc::borrow::Cow;
use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        procfs::{BLOCK_SIZE, ProcFs},
        utils::DirentVisitor,
        vfs::{
            file_system::{FileSystem, SuperBlock},
            inode::{
                Extension, FileOps, Inode, Metadata, MknodType, RenameMode, RevalidationPolicy,
            },
            path::{Dentry, is_dot, is_dotdot},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    thread::Thread,
};

pub(in procfs) struct ProcDir<D: ProcDirOps> {
    inner: D,
    this: Weak<ProcDir<D>>,
    parent: Option<Weak<dyn Inode>>,
    common: Common,
}

impl<D: ProcDirOps> ProcDir<D> {
    pub(in procfs) fn new_root(
        dir: D,
        fs: Weak<dyn FileSystem>,
        ino: u64,
        sb: &SuperBlock,
        mode: InodeMode,
    ) -> Arc<Self> {
        let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, sb.container_dev_id);
        let common = Common::new(metadata, fs);

        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent: None,
            common,
        })
    }

    pub(in procfs) fn new(dir: D, parent: Weak<dyn Inode>, mode: InodeMode) -> Arc<Self> {
        let common = {
            let fs = parent.upgrade().unwrap().fs();
            let procfs = fs.downcast_ref::<ProcFs>().unwrap();
            let ino = procfs.alloc_id();
            let metadata = Metadata::new_dir(ino, mode, BLOCK_SIZE, procfs.sb().container_dev_id);
            Common::new(metadata, Arc::downgrade(&fs))
        };
        Arc::new_cyclic(|weak_self| Self {
            inner: dir,
            this: weak_self.clone(),
            parent: Some(parent),
            common,
        })
    }

    fn this(&self) -> Arc<ProcDir<D>> {
        self.this.upgrade().unwrap()
    }

    pub(in procfs) fn this_weak(&self) -> &Weak<ProcDir<D>> {
        &self.this
    }

    fn parent(&self) -> Option<Arc<dyn Inode>> {
        self.parent.as_ref().and_then(|p| p.upgrade())
    }

    pub(in procfs) fn inner(&self) -> &D {
        &self.inner
    }
}

impl<D: ProcDirOps + 'static> FileOps for ProcDir<D> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EISDIR))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EISDIR))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        fn special_entries<D: ProcDirOps + 'static>(
            dir: &ProcDir<D>,
        ) -> impl Iterator<Item = (&'static str, Arc<dyn Inode>, usize)> {
            let this_inode: Arc<dyn Inode> = dir.this();
            let parent_inode = dir.parent().unwrap_or_else(|| this_inode.clone());
            [(".", this_inode, 1), ("..", parent_inode, 2)].into_iter()
        }

        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            for (name, inode, next_offset) in special_entries(self) {
                if next_offset <= *offset {
                    continue;
                }

                visitor.visit(name, inode.ino(), inode.type_(), next_offset)?;
                *offset = next_offset;
            }

            self.inner.visit_entries_from_offset(*offset, |child| {
                visitor.visit(
                    child.name.as_ref(),
                    child.ino,
                    child.type_,
                    child.next_offset,
                )?;
                *offset = child.next_offset;
                Ok(())
            })?;
            Ok(())
        };

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(e) if iterate_offset == offset => Err(e),
            _ => Ok(iterate_offset - offset),
        }
    }
}

#[inherit_methods(from = "self.common")]
impl<D: ProcDirOps + 'static> Inode for ProcDir<D> {
    fn size(&self) -> usize;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, self_dentry: &Dentry, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, self_dentry: &Dentry, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, self_dentry: &Dentry, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn metadata(&self) -> Result<Metadata> {
        let owner_thread = self.inner.owner_thread();
        Ok(self.common.metadata_with_owner(owner_thread))
    }

    fn resize(&self, _self_dentry: &Dentry, _new_size: usize) -> Result<()> {
        Err(errno!(EISDIR))
    }

    fn type_(&self) -> InodeType {
        InodeType::Dir
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _type_: InodeType,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(EPERM))
    }

    fn mknod(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _mode: InodeMode,
        _type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(EPERM))
    }

    fn link(&self, _self_dentry: &Dentry, _old_dentry: &Dentry, _name: &str) -> Result<()> {
        Err(errno!(EPERM))
    }

    fn unlink(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(errno!(EPERM))
    }

    fn rmdir(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(errno!(EPERM))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if is_dot(name) {
            return Ok(self.this());
        }
        if is_dotdot(name) {
            return Ok(self.parent().unwrap_or(self.this()));
        }

        self.inner.lookup_child(self, name)
    }

    fn rename(
        &self,
        _old_child_dentry: &Dentry,
        _new_dir_dentry: &Dentry,
        _new_name: &str,
        _replaced_inode: Option<&Arc<dyn Inode>>,
        _mode: RenameMode,
    ) -> Result<()> {
        Err(errno!(EPERM))
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        self.inner.revalidation_policy()
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        self.inner.revalidate_exists(name, child)
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        self.inner.revalidate_absent(name)
    }

    fn seek_end(&self) -> Option<usize> {
        Some(0)
    }
}

pub(in procfs) trait ProcDirOps: Sync + Send + Sized {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        None
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>>;

    fn visit_entries_from_offset<'a, F>(&'a self, _offset: usize, _visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        Ok(())
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::empty()
    }

    #[must_use]
    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        true
    }

    #[must_use]
    fn revalidate_absent(&self, _name: &str) -> bool {
        true
    }
}

pub(in procfs) type StaticDirEntry<Fp> = (&'static str, InodeType, Fp);

pub(in procfs) fn lookup_child_from_table<Fp, F>(
    name: &str,
    table: &[StaticDirEntry<Fp>],
    constructor_adaptor: F,
) -> Option<Arc<dyn Inode>>
where
    Fp: Copy,
    F: FnOnce(Fp) -> Arc<dyn Inode>,
{
    table
        .iter()
        .find(|(child_name, _, _)| *child_name == name)
        .map(|(_, _, child_constructor)| (constructor_adaptor)(*child_constructor))
}

pub(in procfs) fn listed_entries_from_table<'a, Fp>(
    table: &'a [StaticDirEntry<Fp>],
) -> impl Iterator<Item = ListedEntry<'a>> + 'a
where
    Fp: Copy + 'a,
{
    table
        .iter()
        .map(|(name, type_, _)| ListedEntry::new(*name, *type_))
}

mod readdir {

    use super::*;

    pub(in procfs) struct ListedEntry<'a> {
        pub(super) name: Cow<'a, str>,
        pub(super) type_: InodeType,
    }

    impl<'a> ListedEntry<'a> {
        pub(in procfs) fn new(name: impl Into<Cow<'a, str>>, type_: InodeType) -> Self {
            Self {
                name: name.into(),
                type_,
            }
        }

        pub(in procfs) fn name(&self) -> &str {
            self.name.as_ref()
        }
    }

    pub(in procfs) struct ReaddirEntry<'a> {
        pub(super) name: Cow<'a, str>,
        pub(super) ino: u64,
        pub(super) type_: InodeType,
        pub(super) next_offset: usize,
    }

    impl<'a> ReaddirEntry<'a> {
        pub(super) fn new(
            name: impl Into<Cow<'a, str>>,
            ino: u64,
            type_: InodeType,
            next_offset: usize,
        ) -> Self {
            Self {
                name: name.into(),
                ino,
                type_,
                next_offset,
            }
        }
    }

    pub(super) const PLACEHOLDER_DIRENT_INO: u64 = 2;

    pub(in procfs) fn sequential_readdir_entries<'a, I>(
        offset: usize,
        first_entry_offset: usize,
        entries: I,
    ) -> impl Iterator<Item = ReaddirEntry<'a>>
    where
        I: IntoIterator<Item = ListedEntry<'a>>,
    {
        entries
            .into_iter()
            .enumerate()
            .filter_map(move |(idx, entry)| {
                let next_offset = first_entry_offset.saturating_add(idx).saturating_add(1);
                let ListedEntry { name, type_ } = entry;
                (next_offset > offset)
                    .then(|| ReaddirEntry::new(name, PLACEHOLDER_DIRENT_INO, type_, next_offset))
            })
    }

    pub(in procfs) fn keyed_readdir_entries<'a, I, F>(
        offset: usize,
        first_entry_offset: usize,
        keys: I,
        mut entry_fn: F,
    ) -> impl Iterator<Item = ReaddirEntry<'a>>
    where
        I: IntoIterator<Item = usize>,
        F: FnMut(usize) -> ListedEntry<'a>,
    {
        let start_key = offset.saturating_sub(first_entry_offset);
        keys.into_iter().filter_map(move |key| {
            let next_offset = first_entry_offset.saturating_add(key).saturating_add(1);
            (key >= start_key && next_offset > offset).then(|| {
                let ListedEntry { name, type_ } = entry_fn(key);
                ReaddirEntry::new(name, PLACEHOLDER_DIRENT_INO, type_, next_offset)
            })
        })
    }

    pub(in procfs) fn visit_readdir_entries<'a, I, F>(entries: I, mut visit_fn: F) -> Result<()>
    where
        I: IntoIterator<Item = ReaddirEntry<'a>>,
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        for entry in entries {
            visit_fn(entry)?;
        }

        Ok(())
    }

    pub(in procfs) fn visit_listed_entries<'a, I, F>(
        offset: usize,
        listed: I,
        visit_fn: F,
    ) -> Result<()>
    where
        I: IntoIterator<Item = ListedEntry<'a>>,
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_readdir_entries(sequential_readdir_entries(offset, 2, listed), visit_fn)
    }
}

pub(in procfs) use readdir::{
    ListedEntry, ReaddirEntry, keyed_readdir_entries, sequential_readdir_entries,
    visit_listed_entries, visit_readdir_entries,
};
