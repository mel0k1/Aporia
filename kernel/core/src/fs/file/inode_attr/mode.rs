use aster_systree::SysPerms;

use crate::prelude::*;

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct InodeMode: u16 {
        const S_ISUID = 0o4000;
        const S_ISGID = 0o2000;
        const S_ISVTX = 0o1000;
        const S_IRUSR = 0o0400;
        const S_IWUSR = 0o0200;
        const S_IXUSR = 0o0100;
        const S_IRGRP = 0o0040;
        const S_IWGRP = 0o0020;
        const S_IXGRP = 0o0010;
        const S_IROTH = 0o0004;
        const S_IWOTH = 0o0002;
        const S_IXOTH = 0o0001;
    }
}

impl InodeMode {
    pub(crate) fn is_owner_readable(&self) -> bool {
        self.contains(Self::S_IRUSR)
    }

    pub(crate) fn is_owner_writable(&self) -> bool {
        self.contains(Self::S_IWUSR)
    }

    pub(crate) fn is_owner_executable(&self) -> bool {
        self.contains(Self::S_IXUSR)
    }

    pub(crate) fn is_group_readable(&self) -> bool {
        self.contains(Self::S_IRGRP)
    }

    pub(crate) fn is_group_writable(&self) -> bool {
        self.contains(Self::S_IWGRP)
    }

    pub(crate) fn is_group_executable(&self) -> bool {
        self.contains(Self::S_IXGRP)
    }

    pub(crate) fn is_other_readable(&self) -> bool {
        self.contains(Self::S_IROTH)
    }

    pub(crate) fn is_other_writable(&self) -> bool {
        self.contains(Self::S_IWOTH)
    }

    pub(crate) fn is_other_executable(&self) -> bool {
        self.contains(Self::S_IXOTH)
    }

    pub(crate) fn has_sticky_bit(&self) -> bool {
        self.contains(Self::S_ISVTX)
    }

    pub(crate) fn has_set_uid(&self) -> bool {
        self.contains(Self::S_ISUID)
    }

    pub(crate) fn has_set_gid(&self) -> bool {
        self.contains(Self::S_ISGID)
    }
}

impl From<SysPerms> for InodeMode {
    fn from(value: SysPerms) -> Self {
        InodeMode::from_bits_truncate(value.bits())
    }
}

macro_rules! chmod {
    ($mode:expr) => { $mode };

    ($mode:expr, $who:ident + $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::file::chmod!(@apply $mode, $who, '+', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident - $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::file::chmod!(@apply $mode, $who, '-', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident = $perms:ident $(, $($rest:tt)*)?) => {
        $crate::fs::file::chmod!(@apply $mode, $who, '=', $perms $(, $($rest)*)?)
    };
    ($mode:expr, $who:ident = $(, $($rest:tt)*)?) => {
        $crate::fs::file::chmod!(@apply $mode, $who, '=', none $(, $($rest)*)?)
    };

    (@apply $mode:expr, $who:ident, $op:expr, $perms:ident $(, $($rest:tt)*)?) => {{
        let mask = $crate::fs::file::who_and_perms_to_mask!($who, $perms);
        let new_mode = match $op {
            '+' => $mode | mask,
            '-' => $mode & !mask,
            '=' => ($mode & !$crate::fs::file::who_and_perms_to_mask!($who, rwx)) | mask,
            _ => unreachable!(),
        };
        $crate::fs::file::chmod!(new_mode $(, $($rest)*)?)
    }};
}

macro_rules! mkmod {
    ($($args:tt)*) => {
        $crate::fs::file::chmod!($crate::fs::file::InodeMode::empty(), $($args)*)
    };
}

macro_rules! who_and_perms_to_mask {
    ($who:ident, $perms:ident) => {
        $crate::fs::file::InodeMode::from_bits_truncate(
            $crate::fs::file::who_to_mask!($who) & $crate::fs::file::perms_to_mask!($perms),
        )
    };
}

macro_rules! who_to_mask {
    (u) => {
        0o700
    };
    (g) => {
        0o070
    };
    (o) => {
        0o007
    };
    (ug) => {
        0o770
    };
    (uo) => {
        0o707
    };
    (go) => {
        0o077
    };
    (a) => {
        0o777
    };
    (ugo) => {
        0o777
    };
}

macro_rules! perms_to_mask {
    (none) => {
        0
    };
    (r) => {
        0o444
    };
    (w) => {
        0o222
    };
    (x) => {
        0o111
    };
    (rw) => {
        0o666
    };
    (rx) => {
        0o555
    };
    (wx) => {
        0o333
    };
    (rwx) => {
        0o777
    };
}

pub(crate) use chmod;
pub(crate) use mkmod;
pub(crate) use perms_to_mask;
pub(crate) use who_and_perms_to_mask;
pub(crate) use who_to_mask;

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    #[ktest]
    fn mkmod_and_chmod() {
        let mode0 = mkmod!(a+rw);
        assert!(mode0.bits() == 0o666);

        let mode1 = chmod!(mode0, u+x);
        assert!(mode1.bits() == 0o766);

        let mode2 = mkmod!(a+rw, u+x);
        assert!(mode2.bits() == 0o766);

        let mode3 = chmod!(mode2, ug-wx);
        assert!(mode3.bits() == 0o446);

        let mode4 = chmod!(mode3, o=rx);
        assert!(mode4.bits() == 0o445);

        let mode5 = chmod!(mode4, u=, g=, o=);
        assert!(mode5.bits() == 0o000);
    }
}
