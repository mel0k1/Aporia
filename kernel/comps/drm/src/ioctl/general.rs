use aster_core::prelude::*;
use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use crate::{device::DrmFeatures, file::DrmFile, ioctl::ioctl_defs};

impl DrmFile {
    pub(super) fn drm_get_version(&self, cmd: ioctl_defs::Version) -> Result<i32> {
        let device = self.device();
        let name = device.name();
        let desc = device.desc();

        let date = "0";
        let major = 0;
        let minor = 0;
        let patch_level = 0;

        cmd.with_data_ptr(|args_ptr| {
            let mut args: DrmVersion = args_ptr.read()?;
            let userspace = args_ptr.vm();

            args.version_major = major;
            args.version_minor = minor;
            args.version_patchlevel = patch_level;

            let copy_result: Result<()> = (|| {
                copy_drm_field(&userspace, args.name, &mut args.name_len, name.as_bytes())?;
                copy_drm_field(&userspace, args.date, &mut args.date_len, date.as_bytes())?;
                copy_drm_field(&userspace, args.desc, &mut args.desc_len, desc.as_bytes())?;
                Ok(())
            })();

            args_ptr.write(&args)?;
            copy_result
        })?;

        Ok(0)
    }

    pub(super) fn drm_get_unique(&self, cmd: ioctl_defs::GetUnique) -> Result<i32> {
        let mut args: DrmUnique = cmd.read()?;

        args.unique_len = 0;
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_magic(&self, cmd: ioctl_defs::GetMagic) -> Result<i32> {
        let args = DrmAuth {
            magic: self.get_or_allocate_magic()?,
        };
        cmd.write(&args)?;
        Ok(0)
    }

    pub(super) fn drm_get_cap(&self, cmd: ioctl_defs::GetCap) -> Result<i32> {
        let mut args: DrmGetCap = cmd.read()?;
        let Ok(cap) = DrmGetCapability::try_from(args.capability) else {
            return Err(errno!(EINVAL, "the DRM device capability is unknown"));
        };
        let device = self.device();

        let value = match cap {
            DrmGetCapability::TimestampMonotonic => 1,
            DrmGetCapability::Prime => 0,
            DrmGetCapability::SyncObj => device.has_features(DrmFeatures::SYNCOBJ) as u64,
            DrmGetCapability::SyncObjTimeline => {
                device.has_features(DrmFeatures::SYNCOBJ_TIMELINE) as u64
            }
            _ => self.kms_get_cap(cap)?,
        };

        args.value = value;

        cmd.write(&args)?;
        Ok(0)
    }

    fn kms_get_cap(&self, cap: DrmGetCapability) -> Result<u64> {
        let device = self.device();
        let kms_ops = device
            .as_kms_ops()
            .ok_or_else(|| errno!(EOPNOTSUPP, "the DRM device lacks modesetting"))?;
        let mode_config = kms_ops.mode_config();

        let result = match cap {
            DrmGetCapability::DumbBuffer => device.as_gem_ops().is_some() as u64,
            DrmGetCapability::VblankHighCrtc => 1,
            DrmGetCapability::DumbPreferredDepth => {
                u64::from(mode_config.preferred_dumb_buffer_depth())
            }
            DrmGetCapability::DumbPreferShadow => mode_config.prefer_shadow_buffer() as u64,
            DrmGetCapability::AsyncPageFlip => mode_config.supports_async_page_flip() as u64,
            DrmGetCapability::CursorWidth => {
                const DRM_DEFAULT_CURSOR_WIDTH: u64 = 64;
                mode_config
                    .cursor_size()
                    .map_or(DRM_DEFAULT_CURSOR_WIDTH, |size| u64::from(size.width()))
            }
            DrmGetCapability::CursorHeight => {
                const DRM_DEFAULT_CURSOR_HEIGHT: u64 = 64;
                mode_config
                    .cursor_size()
                    .map_or(DRM_DEFAULT_CURSOR_HEIGHT, |size| u64::from(size.height()))
            }
            DrmGetCapability::Addfb2Modifiers => mode_config.supports_fb_modifiers() as u64,
            DrmGetCapability::PageFlipTarget => 0,
            DrmGetCapability::CrtcInVblankEvent => 1,
            DrmGetCapability::AtomicAsyncPageFlip => 0,
            _ => 0,
        };

        Ok(result)
    }

    pub(super) fn drm_auth_magic(&self, cmd: ioctl_defs::AuthMagic) -> Result<i32> {
        let args: DrmAuth = cmd.read()?;
        self.authenticate_magic(args.magic)?;
        Ok(0)
    }

    pub(super) fn drm_set_master(&self, _cmd: ioctl_defs::SetMaster) -> Result<i32> {
        self.set_master()?;
        Ok(0)
    }

    pub(super) fn drm_drop_master(&self, _cmd: ioctl_defs::DropMaster) -> Result<i32> {
        self.drop_master()?;
        Ok(0)
    }
}

fn copy_drm_field(
    userspace: &impl VmIo,
    user_addr: usize,
    user_capacity: &mut usize,
    value: &[u8],
) -> Result<()> {
    let copy_len = core::cmp::min(*user_capacity, value.len());
    *user_capacity = value.len();

    if user_addr != 0 && copy_len != 0 {
        userspace.write_bytes(user_addr, &value[..copy_len])?;
    }

    Ok(())
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmVersion {
    version_major: i32,
    version_minor: i32,
    version_patchlevel: i32,

    name_len: usize,
    name: usize,
    date_len: usize,
    date: usize,
    desc_len: usize,
    desc: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmUnique {
    unique_len: usize,
    unique: usize,
}

#[repr(u64)]
#[derive(Debug, TryFromInt)]
enum DrmGetCapability {
    DumbBuffer = 0x1,
    VblankHighCrtc = 0x2,
    DumbPreferredDepth = 0x3,
    DumbPreferShadow = 0x4,
    Prime = 0x5,
    TimestampMonotonic = 0x6,
    AsyncPageFlip = 0x7,
    CursorWidth = 0x8,
    CursorHeight = 0x9,
    Addfb2Modifiers = 0x10,
    PageFlipTarget = 0x11,
    CrtcInVblankEvent = 0x12,
    SyncObj = 0x13,
    SyncObjTimeline = 0x14,
    AtomicAsyncPageFlip = 0x15,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmGetCap {
    capability: u64,
    value: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct DrmAuth {
    magic: u32,
}
