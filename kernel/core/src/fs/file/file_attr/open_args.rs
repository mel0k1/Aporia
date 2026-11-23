use crate::{
    fs::file::{AccessMode, CreationFlags, InodeMode, StatusFlags},
    prelude::*,
};

#[derive(Debug)]
pub(crate) struct OpenArgs {
    pub creation_flags: CreationFlags,
    pub status_flags: StatusFlags,
    pub access_mode: AccessMode,
    pub inode_mode: InodeMode,
}

impl OpenArgs {
    pub(crate) fn from_flags_and_mode(flags: u32, inode_mode: InodeMode) -> Result<Self> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;

        if creation_flags.contains(CreationFlags::O_TMPFILE)
            && !status_flags.contains(StatusFlags::O_PATH)
        {
            if !creation_flags.contains(CreationFlags::O_DIRECTORY) {
                return Err(errno!(EINVAL, "O_TMPFILE requires O_DIRECTORY"));
            }
            if !access_mode.is_writable() {
                return Err(errno!(EINVAL, "O_TMPFILE requires O_RDWR or O_WRONLY"));
            }
            if creation_flags.contains(CreationFlags::O_CREAT) {
                return Err(errno!(
                    EINVAL,
                    "O_TMPFILE and O_CREAT are mutually exclusive"
                ));
            }
        } else if creation_flags.contains(CreationFlags::O_CREAT)
            && creation_flags.contains(CreationFlags::O_DIRECTORY)
        {
            return Err(errno!(
                EINVAL,
                "O_CREAT and O_DIRECTORY cannot be specified together"
            ));
        }

        Ok(Self {
            creation_flags,
            status_flags,
            access_mode,
            inode_mode,
        })
    }

    pub(crate) fn from_modes(access_mode: AccessMode, inode_mode: InodeMode) -> Self {
        Self {
            creation_flags: CreationFlags::empty(),
            status_flags: StatusFlags::empty(),
            access_mode,
            inode_mode,
        }
    }

    pub(crate) fn follow_tail_link(&self) -> bool {
        !(self.creation_flags.contains(CreationFlags::O_NOFOLLOW)
            || self.creation_flags.contains(CreationFlags::O_CREAT)
                && self.creation_flags.contains(CreationFlags::O_EXCL))
    }

    pub(crate) fn is_tmpfile(&self) -> bool {
        self.creation_flags.contains(CreationFlags::O_TMPFILE)
            && !self.status_flags.contains(StatusFlags::O_PATH)
    }
}
