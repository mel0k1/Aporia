use aster_rights::Rights;
use bitflags::bitflags;
use ostd::mm::PageFlags;

use crate::prelude::*;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct VmPerms: u32 {
        const READ          = 1 << 0;
        const WRITE         = 1 << 1;
        const EXEC          = 1 << 2;
        const MAY_READ      = 1 << 3;
        const MAY_WRITE     = 1 << 4;
        const MAY_EXEC      = 1 << 5;
        const ALL_PERMS     = Self::READ.bits() | Self::WRITE.bits() | Self::EXEC.bits();
        const ALL_MAY_PERMS = Self::MAY_READ.bits() | Self::MAY_WRITE.bits() | Self::MAY_EXEC.bits();
    }
}

impl VmPerms {
    pub(crate) fn check(&self) -> Result<()> {
        let requested = *self & Self::ALL_PERMS;
        let allowed = VmPerms::from_bits_truncate((*self & Self::ALL_MAY_PERMS).bits() >> 3);
        if !allowed.contains(requested) {
            return Err(errno!(EACCES, "permission denied"));
        }

        Ok(())
    }

    pub(crate) fn from_user_bits(bits: u32) -> Result<Self> {
        if let Some(vm_perms) = VmPerms::from_bits(bits)
            && Self::ALL_PERMS.contains(vm_perms)
        {
            Ok(vm_perms)
        } else {
            Err(errno!(EINVAL, "invalid permissions"))
        }
    }

    pub(crate) fn from_user_bits_truncate(bits: u32) -> Self {
        VmPerms::from_bits_truncate(bits) & Self::ALL_PERMS
    }
}

impl From<Rights> for VmPerms {
    fn from(rights: Rights) -> VmPerms {
        let mut vm_perm = VmPerms::empty();
        if rights.contains(Rights::READ) {
            vm_perm |= VmPerms::READ;
        }
        if rights.contains(Rights::WRITE) {
            vm_perm |= VmPerms::WRITE;
        }
        if rights.contains(Rights::EXEC) {
            vm_perm |= VmPerms::EXEC;
        }
        vm_perm
    }
}

impl From<VmPerms> for Rights {
    fn from(vm_perms: VmPerms) -> Rights {
        let mut rights = Rights::empty();
        if vm_perms.contains(VmPerms::READ) {
            rights |= Rights::READ;
        }
        if vm_perms.contains(VmPerms::WRITE) {
            rights |= Rights::WRITE;
        }
        if vm_perms.contains(VmPerms::EXEC) {
            rights |= Rights::EXEC;
        }
        rights
    }
}

impl From<PageFlags> for VmPerms {
    fn from(flags: PageFlags) -> Self {
        let mut perms = VmPerms::empty();
        if flags.contains(PageFlags::R) {
            perms |= VmPerms::READ;
        }
        if flags.contains(PageFlags::W) {
            perms |= VmPerms::WRITE;
        }
        if flags.contains(PageFlags::X) {
            perms |= VmPerms::EXEC;
        }
        perms
    }
}

impl From<VmPerms> for PageFlags {
    fn from(val: VmPerms) -> Self {
        let mut flags = PageFlags::empty();
        if val.contains(VmPerms::READ) {
            flags |= PageFlags::R;
        }
        if val.contains(VmPerms::WRITE) {
            flags |= PageFlags::W;
        }
        if val.contains(VmPerms::EXEC) {
            flags |= PageFlags::X;
        }
        flags
    }
}
