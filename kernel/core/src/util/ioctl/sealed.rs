use int_to_c_enum::TryFromInt;

#[derive(Clone, Copy, Debug)]
pub(super) struct IoctlCmd(u32);

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum IoctlDir {
    None = 0,
    Write = 1,
    Read = 2,
    ReadWrite = 3,
}

impl IoctlCmd {
    pub(super) const fn new(cmd: u32) -> Self {
        Self(cmd)
    }

    pub(super) const fn as_u32(self) -> u32 {
        self.0
    }

    pub(super) const fn nr(self) -> u8 {
        self.0 as u8
    }

    pub(super) const fn set_nr(&mut self, nr: u8) {
        self.0 = (self.0 & !0xFF) | (nr as u32);
    }

    pub(super) const fn magic(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub(super) const fn size(self) -> u16 {
        ((self.0 >> 16) as u16) & 0x3FFF
    }

    pub(super) fn dir(self) -> IoctlDir {
        IoctlDir::try_from((self.0 >> 30) as u8).unwrap()
    }
}

pub trait DataSpec {
    const SIZE: Option<u16>;
    const DIR: IoctlDir;
}

pub trait PtrDataSpec: DataSpec {
    type Pointee;
}
