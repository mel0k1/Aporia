use bitflags::bitflags;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file::file_table::{RawFileDesc, get_file_fast},
        utils::PATH_MAX,
        vfs::{
            file_system::FsFlags,
            path::{FsPath, Mount, PerMountFlags},
        },
    },
    prelude::*,
};

pub(super) fn sys_statfs(
    path_ptr: Vaddr,
    statfs_buf_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path_name = user_space.read_cstring(path_ptr, PATH_MAX)?;
    debug!(
        "path = {:?}, statfs_buf_ptr = 0x{:x}",
        path_name, statfs_buf_ptr,
    );

    let statfs = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::try_from(path_name.as_ref())?;
        let path = ctx
            .thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?;
        Statfs::new(path.mount_node())
    };
    user_space.write_val(statfs_buf_ptr, &statfs)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_fstatfs(
    raw_fd: RawFileDesc,
    statfs_buf_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "raw_fd = {}, statfs_buf_addr = 0x{:x}",
        raw_fd, statfs_buf_ptr
    );

    let statfs = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
        Statfs::new(file.path().mount_node())
    };
    ctx.user_space().write_val(statfs_buf_ptr, &statfs)?;
    Ok(SyscallReturn::Return(0))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct Statfs {
    f_type: u64,
    f_bsize: usize,
    f_blocks: usize,
    f_bfree: usize,
    f_bavail: usize,
    f_files: usize,
    f_ffree: usize,
    f_fsid: u64,
    f_namelen: usize,
    f_frsize: usize,
    f_flags: u64,
    f_spare: [u64; 4],
}

impl Statfs {
    fn new(mount: &Mount) -> Self {
        let sb = mount.fs().sb();
        let statfs_flags =
            StatfsFlags::new(mount.flags(), FsFlags::from_bits_truncate(sb.flags as u32));
        Self {
            f_type: sb.magic,
            f_bsize: sb.bsize,
            f_blocks: sb.blocks,
            f_bfree: sb.bfree,
            f_bavail: sb.bavail,
            f_files: sb.files,
            f_ffree: sb.ffree,
            f_fsid: sb.fsid,
            f_namelen: sb.namelen,
            f_frsize: sb.frsize,
            f_flags: statfs_flags.bits() as u64,
            f_spare: [0u64; 4],
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct StatfsFlags: u32 {
        const ST_RDONLY      = 1 << 0;
        const ST_NOSUID      = 1 << 1;
        const ST_NODEV       = 1 << 2;
        const ST_NOEXEC      = 1 << 3;
        const ST_SYNCHRONOUS = 1 << 4;
        const ST_MANDLOCK    = 1 << 6;
        const ST_NOATIME     = 1 << 10;
        const ST_NODIRATIME  = 1 << 11;
        const ST_RELATIME    = 1 << 12;
        const ST_NOSYMFOLLOW = 1 << 13;
    }
}

impl StatfsFlags {
    fn new(per_mount_flags: PerMountFlags, fs_flags: FsFlags) -> Self {
        let mut statfs_flags = StatfsFlags::from_bits_truncate(per_mount_flags.bits());
        statfs_flags |= StatfsFlags::from_bits_truncate(fs_flags.bits());

        if per_mount_flags.contains(PerMountFlags::NOSYMFOLLOW) {
            statfs_flags |= StatfsFlags::ST_NOSYMFOLLOW;
        }
        if per_mount_flags.contains(PerMountFlags::RELATIME) {
            statfs_flags |= StatfsFlags::ST_RELATIME;
        }

        statfs_flags
    }
}
