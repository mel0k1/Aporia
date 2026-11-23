use core::time::Duration;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file::file_table::RawFileDesc,
        vfs::path::{EmptyPathStr, FsPath, Path},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

const STATX_ATTR_MOUNT_ROOT: u64 = 0x0000_2000;

pub(super) fn sys_statx(
    dirfd: RawFileDesc,
    filename_ptr: Vaddr,
    flags: u32,
    mask: u32,
    statx_buf_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let filename = user_space.read_cstring(filename_ptr, MAX_FILENAME_LEN)?;
    let flags =
        StatxFlags::from_bits(flags).ok_or_else(|| errno!(EINVAL, "invalid statx flags"))?;
    let mask = StatxMask::from_bits_truncate(mask);
    debug!(
        "dirfd = {}, filename = {:?}, flags = {:?}, mask = {:?}, statx_buf_ptr = 0x{:x}",
        dirfd, filename, flags, mask, statx_buf_ptr,
    );

    if flags.contains(StatxFlags::AT_STATX_FORCE_SYNC)
        && flags.contains(StatxFlags::AT_STATX_DONT_SYNC)
    {
        return Err(errno!(EINVAL, "invalid statx flags"));
    }

    if mask.contains(StatxMask::STATX_RESERVED) {
        return Err(errno!(
            EINVAL,
            "mask reserved for future struct statx expansion"
        ));
    }

    let path = {
        let filename = filename.to_string_lossy();
        let fs_path =
            FsPath::from_fd_at(dirfd, &filename, EmptyPathStr::AllowIfFlag(flags.bits()))?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(StatxFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    let statx = Statx::new(&path, mask)?;

    user_space.write_val(statx_buf_ptr, &statx)?;
    Ok(SyscallReturn::Return(0))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct Statx {
    stx_mask: u32,
    stx_blksize: u32,
    stx_attributes: u64,
    stx_nlink: u32,
    stx_uid: u32,
    stx_gid: u32,
    stx_mode: u16,
    __spare0: [u16; 1],
    stx_ino: u64,
    stx_size: u64,
    stx_blocks: u64,
    stx_attributes_mask: u64,
    stx_atime: StatxTimestamp,
    stx_btime: StatxTimestamp,
    stx_ctime: StatxTimestamp,
    stx_mtime: StatxTimestamp,
    stx_rdev_major: u32,
    stx_rdev_minor: u32,
    stx_dev_major: u32,
    stx_dev_minor: u32,
    stx_mnt_id: u64,
    stx_dio_mem_align: u32,
    stx_dio_offset_align: u32,
    __spare3: [u64; 12],
}

impl Statx {
    fn new(path: &Path, requested_mask: StatxMask) -> Result<Self> {
        let info = path.metadata()?;

        let (stx_dev_major, stx_dev_minor) =
            device_id::decode_device_numbers(info.container_dev_id.as_encoded_u64());
        let (stx_rdev_major, stx_rdev_minor) =
            device_id::decode_device_numbers(info.self_dev_id.map_or(0, |id| id.as_encoded_u64()));

        let stx_attributes_mask = STATX_ATTR_MOUNT_ROOT;

        let mut stx_attributes = 0;
        if path.is_mount_root() {
            stx_attributes |= STATX_ATTR_MOUNT_ROOT;
        }

        let want_unique_mnt_id = requested_mask.contains(StatxMask::STATX_MNT_ID_UNIQUE);
        let stx_mnt_id = if want_unique_mnt_id {
            path.mount_node().unique_id()
        } else {
            path.mount_node().id() as u64
        };
        let mnt_id_mask_bit = if want_unique_mnt_id {
            StatxMask::STATX_MNT_ID_UNIQUE
        } else {
            StatxMask::STATX_MNT_ID
        };

        let mut stx_mask = StatxMask::STATX_BASIC_STATS.bits() | mnt_id_mask_bit.bits();

        let stx_btime = if let Some(birth_at) = info.birth_at {
            stx_mask |= StatxMask::STATX_BTIME.bits();
            StatxTimestamp::from(birth_at)
        } else {
            StatxTimestamp::default()
        };

        Ok(Self {
            stx_mask,
            stx_blksize: info.optimal_block_size as u32,
            stx_attributes,
            stx_nlink: info.nr_hard_links as u32,
            stx_uid: info.uid.into(),
            stx_gid: info.gid.into(),
            stx_mode: info.type_ as u16 | info.mode.bits(),
            __spare0: [0; 1],
            stx_ino: info.ino,
            stx_size: info.size as u64,
            stx_blocks: info.nr_sectors_allocated as u64,
            stx_attributes_mask,
            stx_atime: StatxTimestamp::from(info.last_access_at),
            stx_btime,
            stx_ctime: StatxTimestamp::from(info.last_meta_change_at),
            stx_mtime: StatxTimestamp::from(info.last_modify_at),
            stx_rdev_major,
            stx_rdev_minor,
            stx_dev_major,
            stx_dev_minor,
            stx_mnt_id,
            stx_dio_mem_align: 0,
            stx_dio_offset_align: 0,
            __spare3: [0; 12],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct StatxTimestamp {
    tv_sec: i64,
    tv_nsec: u32,
    __reserved: i32,
}

impl From<Duration> for StatxTimestamp {
    fn from(duration: Duration) -> Self {
        Self {
            tv_sec: duration.as_secs() as i64,
            tv_nsec: duration.subsec_nanos(),
            __reserved: 0,
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct StatxFlags: u32 {
        const AT_EMPTY_PATH         = 1 << 12;
        const AT_NO_AUTOMOUNT       = 1 << 11;
        const AT_SYMLINK_NOFOLLOW   = 1 << 8;
        const AT_STATX_SYNC_AS_STAT = 0;
        const AT_STATX_FORCE_SYNC   = 1 << 13;
        const AT_STATX_DONT_SYNC    = 1 << 14;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct StatxMask: u32 {
        const STATX_TYPE           = 0x00000001;
        const STATX_MODE           = 0x00000002;
        const STATX_NLINK          = 0x00000004;
        const STATX_UID            = 0x00000008;
        const STATX_GID            = 0x00000010;
        const STATX_ATIME          = 0x00000020;
        const STATX_MTIME          = 0x00000040;
        const STATX_CTIME          = 0x00000080;
        const STATX_INO            = 0x00000100;
        const STATX_SIZE           = 0x00000200;
        const STATX_BLOCKS         = 0x00000400;
        const STATX_BASIC_STATS    = 0x000007ff;
        const STATX_BTIME          = 0x00000800;
        const STATX_MNT_ID         = 0x00001000;
        const STATX_DIOALIGN       = 0x00002000;
        const STATX_MNT_ID_UNIQUE  = 0x00004000;
        const STATX_SUBVOL         = 0x00008000;
        const STATX_WRITE_ATOMIC   = 0x00010000;
        const STATX_DIO_READ_ALIGN = 0x00020000;
        const STATX_RESERVED       = 0x80000000;
        const STATX_ALL            = 0x00000fff;
    }
}
