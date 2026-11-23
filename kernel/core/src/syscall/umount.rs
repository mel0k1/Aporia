use super::SyscallReturn;
use crate::{
    fs::vfs::path::{AT_FDCWD, EmptyPathStr, FsPath},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub(super) fn sys_umount(path_addr: Vaddr, flags: u64, ctx: &Context) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    let umount_flags = UmountFlags::from_bits_truncate(flags as u32);
    debug!("path = {:?}, flags = {:?}", path_name, umount_flags);

    umount_flags.check_unsupported_flags()?;

    let path_name = path_name.to_string_lossy();
    let fs_path = FsPath::from_fd_at(AT_FDCWD, &path_name, EmptyPathStr::Reject)?;

    let target_path = if umount_flags.contains(UmountFlags::UMOUNT_NOFOLLOW) {
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?
    } else {
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    super::mount::check_mount_api_capability(ctx)?;

    target_path.get_top_path().unmount(ctx)?;

    Ok(SyscallReturn::Return(0))
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct UmountFlags: u32 {
        const MNT_FORCE       = 0x00000001;
        const MNT_DETACH      = 0x00000002;
        const MNT_EXPIRE      = 0x00000004;
        const UMOUNT_NOFOLLOW = 0x00000008;
    }
}

impl UmountFlags {
    fn check_unsupported_flags(&self) -> Result<()> {
        let supported_flags = UmountFlags::MNT_FORCE
            | UmountFlags::MNT_DETACH
            | UmountFlags::MNT_EXPIRE
            | UmountFlags::UMOUNT_NOFOLLOW;
        let unsupported_flags = *self - supported_flags;
        if !unsupported_flags.is_empty() {
            return Err(errno!(EINVAL, "unsupported flags"));
        }
        Ok(())
    }
}
