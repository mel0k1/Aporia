use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::vfs::path::MNT_UNIQUE_ID_MIN, prelude::*};

const MNT_ID_REQ_SIZE_VER0: u32 = 24;

const LSMT_ROOT: u64 = u64::MAX;

const LISTMOUNT_REVERSE: u32 = 1 << 0;

const LISTMOUNT_NR_LIMIT: usize = 1_000_000;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct MntIdReq {
    size: u32,
    spare: u32,
    mnt_id: u64,
    param: u64,
}

const _: () = assert!(size_of::<MntIdReq>() as u32 == MNT_ID_REQ_SIZE_VER0);

pub(super) fn sys_listmount(
    req_ptr: Vaddr,
    mnt_ids_ptr: Vaddr,
    nr_mnt_ids: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "req_ptr = 0x{:x}, mnt_ids_ptr = 0x{:x}, nr_mnt_ids = {}, flags = 0x{:x}",
        req_ptr, mnt_ids_ptr, nr_mnt_ids, flags,
    );

    if flags & !LISTMOUNT_REVERSE != 0 {
        return Err(errno!(EINVAL, "invalid listmount flags"));
    }
    let reverse = flags & LISTMOUNT_REVERSE != 0;

    if nr_mnt_ids > LISTMOUNT_NR_LIMIT {
        return Err(errno!(EOVERFLOW, "nr_mnt_ids too large"));
    }

    let user_space = ctx.user_space();

    let mut mnt_ids_writer = user_space.writer(mnt_ids_ptr, nr_mnt_ids * size_of::<u64>())?;

    let req: MntIdReq = user_space.read_val(req_ptr)?;
    if req.size != MNT_ID_REQ_SIZE_VER0 {
        return Err(errno!(EINVAL, "unsupported mnt_id_req size"));
    }
    if req.spare != 0 {
        return Err(errno!(EINVAL, "non-zero spare field"));
    }
    if req.mnt_id != LSMT_ROOT && req.mnt_id < MNT_UNIQUE_ID_MIN {
        return Err(errno!(EINVAL, "mnt_id is not a valid unique mount ID"));
    }
    if req.param != 0 && req.param < MNT_UNIQUE_ID_MIN {
        return Err(errno!(EINVAL, "param is not a valid unique mount ID"));
    }

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let mnt_ns = ns_proxy.mnt_ns();

    let parent_mount = if req.mnt_id == LSMT_ROOT {
        mnt_ns.root().clone()
    } else {
        mnt_ns
            .lookup_by_unique_id(req.mnt_id)
            .ok_or_else(|| errno!(ENOENT, "no such mount"))?
    };

    if nr_mnt_ids == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let mounts = mnt_ns.descendant_mounts_of(&parent_mount);
    let ids: Vec<_> = if reverse {
        mounts
            .rev()
            .map(|mount| mount.unique_id())
            .filter(|&id| req.param == 0 || id < req.param)
            .take(nr_mnt_ids)
            .collect()
    } else {
        mounts
            .map(|mount| mount.unique_id())
            .filter(|&id| id > req.param)
            .take(nr_mnt_ids)
            .collect()
    };

    for id in &ids {
        mnt_ids_writer.write_val(id)?;
    }

    Ok(SyscallReturn::Return(ids.len() as isize))
}
