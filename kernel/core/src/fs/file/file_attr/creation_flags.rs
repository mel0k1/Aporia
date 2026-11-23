use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct CreationFlags: u32 {
        const O_CREAT     = 1 << 6;
        const O_EXCL      = 1 << 7;
        const O_NOCTTY    = 1 << 8;
        const O_TRUNC     = 1 << 9;
        const O_DIRECTORY = 1 << 16;
        const O_NOFOLLOW  = 1 << 17;
        const O_CLOEXEC   = 1 << 19;
        const O_TMPFILE   = 1 << 22;
    }
}
