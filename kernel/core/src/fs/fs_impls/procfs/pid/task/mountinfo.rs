use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::{
            file_system::FsFlags,
            inode::Inode,
            path::{Mount, Path, PathResolver, PerMountFlags, RecyclableMountId},
        },
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
};

pub(super) fn make_mount_point_path(
    is_resolver_root_mount: bool,
    parent: Option<&Arc<Mount>>,
    mount: &Mount,
    path_resolver: &PathResolver,
) -> String {
    if is_resolver_root_mount {
        "/".to_string()
    } else if let Some(parent) = parent {
        if let Some(mount_point_dentry) = mount.mountpoint() {
            path_resolver
                .make_abs_path(&Path::new(parent.clone(), mount_point_dentry))
                .into_string()
        } else {
            "".to_string()
        }
    } else {
        "/".to_string()
    }
}

struct MountInfoEntry<'a> {
    mount_id: RecyclableMountId,
    parent_id: RecyclableMountId,
    major: u32,
    minor: u32,
    root: &'a str,
    mount_point: &'a str,
    mount_flags: PerMountFlags,
    propagation_info: &'a str,
    fs_type: &'a str,
    source: &'a str,
    fs_flags: FsFlags,
}

impl core::fmt::Display for MountInfoEntry<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} {} {}:{} {} {} {}{} - {} {} {}",
            self.mount_id,
            self.parent_id,
            self.major,
            self.minor,
            self.root,
            self.mount_point,
            self.mount_flags,
            self.propagation_info,
            self.fs_type,
            self.source,
            self.fs_flags,
        )
    }
}

pub(super) struct MountInfoFileOps(TidDirOps);

impl MountInfoFileOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }

    fn read_mount_info(
        &self,
        path_resolver: &PathResolver,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for mount in path_resolver.collect_visible_mounts() {
            let mount_id = mount.id();
            let parent = mount.parent().and_then(|parent| parent.upgrade());
            let parent_id = parent.as_ref().map_or(mount_id, |p| p.id());
            let container_dev_id = mount.fs().sb().container_dev_id;
            let major = container_dev_id.major().get() as u32;
            let minor = container_dev_id.minor().get();
            let is_resolver_root_mount = Arc::ptr_eq(&mount, path_resolver.root().mount_node());
            let root = if is_resolver_root_mount {
                path_resolver.root().dentry().path_name()
            } else {
                mount.root_dentry().path_name()
            };
            let mount_point = make_mount_point_path(
                is_resolver_root_mount,
                parent.as_ref(),
                mount.as_ref(),
                path_resolver,
            );
            let mount_flags = mount.flags();
            let propagation_info = mount.propagation_mountinfo_fields();
            let fs_type = mount.fs().name();
            let source = mount.source().unwrap_or("none");
            let fs_flags = mount.fs().flags();

            let entry = MountInfoEntry {
                mount_id,
                parent_id,
                major,
                minor,
                root: &root,
                mount_point: &mount_point,
                mount_flags,
                propagation_info: &propagation_info,
                fs_type,
                source,
                fs_flags,
            };

            writeln!(printer, "{}", entry)?;
        }

        Ok(printer.bytes_written())
    }
}

impl ProcFileOps for MountInfoFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(thread) = self.0.thread() else {
            return Err(errno!(ESRCH, "the thread does not exist"));
        };
        let posix_thread = thread.as_posix_thread().unwrap();

        let fs = posix_thread.read_fs();
        let Some(fs) = fs.as_ref() else {
            return Err(errno!(EINVAL, "the thread has exited"));
        };
        let path_resolver = fs.resolver().read();
        self.read_mount_info(&path_resolver, offset, writer)
    }
}
