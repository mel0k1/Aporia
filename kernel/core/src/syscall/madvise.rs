use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

pub(super) fn sys_madvise(
    addr: Vaddr,
    len: usize,
    behavior: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let behavior = MadviseBehavior::try_from(behavior)?;
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, behavior = {:?}",
        addr, len, behavior
    );

    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(errno!(EINVAL, "the mapping address is not aligned"));
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        return Err(errno!(EINVAL, "the mapping range is not in userspace"));
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    match behavior {
        MadviseBehavior::MADV_DONTNEED => {
            vmar.discard_pages(addr_range)?;
        }
        _ if DUMMY_MADVISE.contains(&behavior) => {
            let query_guard = vmar.query(addr_range);
            if !query_guard.is_fully_mapped() {
                return Err(errno!(
                    ENOMEM,
                    "the range contains pages that are not mapped"
                ));
            }
        }
        _ => return Err(errno!(EINVAL, "the madvise behavior is not supported yet")),
    }

    Ok(SyscallReturn::Return(0))
}

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum MadviseBehavior {
    MADV_NORMAL = 0,
    MADV_RANDOM = 1,
    MADV_SEQUENTIAL = 2,
    MADV_WILLNEED = 3,
    MADV_DONTNEED = 4,

    MADV_FREE = 8,
    MADV_REMOVE = 9,
    MADV_DONTFORK = 10,
    MADV_DOFORK = 11,
    MADV_HWPOISON = 100,
    MADV_SOFT_OFFLINE = 101,

    MADV_MERGEABLE = 12,
    MADV_UNMERGEABLE = 13,

    MADV_HUGEPAGE = 14,
    MADV_NOHUGEPAGE = 15,

    MADV_DONTDUMP = 16,
    MADV_DODUMP = 17,

    MADV_WIPEONFORK = 18,
    MADV_KEEPONFORK = 19,

    MADV_COLD = 20,
    MADV_PAGEOUT = 21,

    MADV_POPULATE_READ = 22,
    MADV_POPULATE_WRITE = 23,

    MADV_DONTNEED_LOCKED = 24,
}

/// Madvise that a dummy implementation is also correct.
///
/// This list can only contain madvise behaviors that do not alter the semantics of the user
/// program. In other words, they are intended solely for performance optimization and can safely
/// be ignored by the kernel.
///
/// **Please think twice before adding a new behavior to this list. Not all madvise behaviors can
/// be no-ops.**
const DUMMY_MADVISE: &[MadviseBehavior] = &[
    MadviseBehavior::MADV_NORMAL,
    MadviseBehavior::MADV_RANDOM,
    MadviseBehavior::MADV_SEQUENTIAL,
    MadviseBehavior::MADV_WILLNEED,
    MadviseBehavior::MADV_FREE,
    MadviseBehavior::MADV_MERGEABLE,
    MadviseBehavior::MADV_UNMERGEABLE,
    MadviseBehavior::MADV_HUGEPAGE,
    MadviseBehavior::MADV_NOHUGEPAGE,
];
