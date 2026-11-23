use core::marker::PhantomData;

use aster_util::safe_ptr::SafePtr;
use sealed::{DataSpec, IoctlCmd, IoctlDir, PtrDataSpec};

use crate::{context::current_userspace, prelude::*};

pub(crate) mod common_defs;
mod sealed;

#[derive(Clone, Copy, Debug)]
pub struct RawIoctl {
    cmd: u32,
    arg: usize,
}

impl RawIoctl {
    pub const fn new(cmd: u32, arg: usize) -> Self {
        Self { cmd, arg }
    }

    pub const fn cmd(self) -> u32 {
        self.cmd
    }

    pub const fn arg(self) -> usize {
        self.arg
    }
}

pub struct Ioctl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D> {
    cmd: IoctlCmd,
    arg: usize,
    _phantom: PhantomData<D>,
}

#[macro_export]
macro_rules! ioc {
    ($linux_name:ident, $raw:literal, $data:ty) => {
        $crate::util::ioctl::Ioctl::<
            { $crate::util::ioctl::magic_and_nr_from_cmd($raw).0 },
            { $crate::util::ioctl::magic_and_nr_from_cmd($raw).1 },
            { false },
            $data,
        >
    };
    ($linux_name:ident, $magic:literal, $nr:literal, $data:ty) => {
        $crate::util::ioctl::Ioctl::<{ $magic }, { $nr }, { true }, $data>
    };
}

#[doc(hidden)]
pub const fn magic_and_nr_from_cmd(raw_cmd: u16) -> (u8, u8) {
    let cmd = IoctlCmd::new(raw_cmd as u32);
    (cmd.magic(), cmd.nr())
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D: DataSpec>
    Ioctl<MAGIC, NR, IS_MODERN, D>
{
    pub fn try_from_raw(raw_ioctl: RawIoctl) -> Option<Self> {
        let cmd = IoctlCmd::new(raw_ioctl.cmd);

        if cmd.magic() != MAGIC {
            return None;
        }
        if cmd.nr() != NR {
            return None;
        }

        if IS_MODERN {
            if let Some(size) = D::SIZE
                && cmd.size() != size
            {
                return None;
            }
            if cmd.dir() != D::DIR {
                return None;
            }
        } else {
            if cmd.size() != 0 || cmd.dir() != IoctlDir::None {
                return None;
            }
        }

        Some(Self {
            cmd,
            arg: raw_ioctl.arg,
            _phantom: PhantomData,
        })
    }
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, D: PtrDataSpec>
    Ioctl<MAGIC, NR, IS_MODERN, D>
{
    fn with_data_ptr_unchecked_access<F, R>(&self, f: F) -> R
    where
        F: for<'a> FnOnce(SafePtr<D::Pointee, CurrentUserSpace<'a>>) -> R,
    {
        f(SafePtr::new(current_userspace!(), self.arg))
    }
}

pub struct NoData;

impl DataSpec for NoData {
    const SIZE: Option<u16> = Some(0);
    const DIR: IoctlDir = IoctlDir::None;
}

pub struct InData<T: ?Sized, P = PassByPtr>(PhantomData<T>, PhantomData<P>);

impl<T, P> DataSpec for InData<T, P> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::Write;
}

impl<T> PtrDataSpec for InData<T, PassByPtr> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, InData<T, PassByPtr>>
{
    pub fn read(&self) -> Result<T> {
        Ok(self.with_data_ptr_unchecked_access(|ptr| ptr.read())?)
    }
}

macro_rules! impl_get_by_val_for {
    { $( $ty:ident )* } => {
        $(
            impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
                Ioctl<MAGIC, NR, IS_MODERN, InData<$ty, PassByVal>>
            {
                pub fn get(&self) -> $ty {
                    self.arg as $ty
                }
            }
        )*
    };
}

impl_get_by_val_for! { i32 u32 }

impl DataSpec for InData<[u8]> {
    const SIZE: Option<u16> = None;
    const DIR: IoctlDir = IoctlDir::Write;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
    Ioctl<MAGIC, NR, IS_MODERN, InData<[u8]>>
{
    pub fn with_reader<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(VmReader<'a>) -> Result<R>,
    {
        f(current_userspace!().reader(self.arg, self.cmd.size() as usize)?)
    }
}

pub struct OutData<T: ?Sized>(PhantomData<T>);

impl<T> DataSpec for OutData<T> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::Read;
}

impl<T> PtrDataSpec for OutData<T> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, OutData<T>>
{
    pub fn write(&self, val: &T) -> Result<()> {
        self.with_data_ptr_unchecked_access(|ptr| ptr.write(val))?;
        Ok(())
    }
}

impl DataSpec for OutData<[u8]> {
    const SIZE: Option<u16> = None;
    const DIR: IoctlDir = IoctlDir::Read;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool>
    Ioctl<MAGIC, NR, IS_MODERN, OutData<[u8]>>
{
    pub fn with_writer<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(VmWriter<'a>) -> Result<R>,
    {
        f(current_userspace!().writer(self.arg, self.cmd.size() as usize)?)
    }
}

pub struct InOutData<T>(PhantomData<T>);

impl<T> DataSpec for InOutData<T> {
    const SIZE: Option<u16> = Some(u16_size_of::<T>());
    const DIR: IoctlDir = IoctlDir::ReadWrite;
}

impl<T> PtrDataSpec for InOutData<T> {
    type Pointee = T;
}

impl<const MAGIC: u8, const NR: u8, const IS_MODERN: bool, T: Pod>
    Ioctl<MAGIC, NR, IS_MODERN, InOutData<T>>
{
    pub fn read(&self) -> Result<T> {
        self.with_data_ptr(|ptr| Ok(ptr.read()?))
    }

    pub fn write(&self, val: &T) -> Result<()> {
        self.with_data_ptr(|ptr| Ok(ptr.write(val)?))
    }

    pub fn with_data_ptr<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(SafePtr<T, CurrentUserSpace<'a>>) -> Result<R>,
    {
        self.with_data_ptr_unchecked_access(f)
    }
}

pub enum PassByVal {}
pub enum PassByPtr {}

const fn u16_size_of<T>() -> u16 {
    let size = size_of::<T>();
    assert!(
        size <= u16::MAX as usize,
        "the type is too large to fit in an ioctl command"
    );
    size as u16
}

#[macro_export]
macro_rules! dispatch_ioctl {
    (
        match $raw:ident {}
    ) => {
        ()
    };

    (
        match $raw:ident {
            _ => $arm:expr $(,)?
        }
    ) => {
        $arm
    };

    (
        match $raw:ident {
            $ty0:ty $(| $ty1:ty)* => $arm:block $(,)?
            $($rest:tt)*
        }
    ) => {
        if <$ty0>::try_from_raw($raw).is_some()
            $(|| <$ty1>::try_from_raw($raw).is_some())*
        {
            $arm
        } else {
            $crate::dispatch_ioctl!(match $raw { $($rest)* })
        }
    };

    (
        match $raw:ident {
            $bind:ident @ $ty:ty => $arm:block $(,)?
            $($rest:tt)*
        }
    ) => {
        if let Some($bind) = <$ty>::try_from_raw($raw) {
            $arm
        } else {
            $crate::dispatch_ioctl!(match $raw { $($rest)* })
        }
    };
}

pub(crate) struct IoctlEnum<const MAGIC: u8, const BASE_NR: u8, const ENUM_MASK: u8, D> {
    base_ioctl: Ioctl<MAGIC, BASE_NR, true, D>,
    discriminant: u8,
}

impl<const MAGIC: u8, const BASE_NR: u8, const ENUM_MASK: u8, D: DataSpec>
    IoctlEnum<MAGIC, BASE_NR, ENUM_MASK, D>
{
    pub(crate) fn try_from_raw(raw_ioctl: RawIoctl) -> Option<Self> {
        const {
            assert!(
                (BASE_NR & ENUM_MASK) == 0,
                "the base NR must not have the mask bits set"
            )
        };

        let (discriminant, raw_base_ioctl) = {
            let mut cmd = IoctlCmd::new(raw_ioctl.cmd);

            let discriminant = cmd.nr() & ENUM_MASK;
            cmd.set_nr(cmd.nr() & !ENUM_MASK);

            (discriminant, RawIoctl::new(cmd.as_u32(), raw_ioctl.arg()))
        };

        let base_ioctl = Ioctl::try_from_raw(raw_base_ioctl)?;

        Some(Self {
            base_ioctl,
            discriminant,
        })
    }
}

impl<const MAGIC: u8, const BASE_NR: u8, const ENUM_MASK: u8, D>
    IoctlEnum<MAGIC, BASE_NR, ENUM_MASK, D>
{
    pub(crate) const fn base_ioctl(&self) -> &Ioctl<MAGIC, BASE_NR, true, D> {
        &self.base_ioctl
    }

    pub(crate) const fn discriminant(&self) -> u8 {
        self.discriminant
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    type SetControlTty = ioc!(TIOCSCTTY, 0x540E, InData<i32, PassByVal>);

    type GetEventBits = IoctlEnum<b'E', 0x20, 0x1F, OutData<[u8]>>;

    #[derive(Debug, Eq, PartialEq)]
    enum Action {
        SetControlTty { should_steal_tty: bool },
        GetEventBits { event_type: u8, buffer_length: u16 },
    }

    impl Action {
        fn parse(cmd: u32, arg: usize) -> Option<Self> {
            let raw_ioctl = RawIoctl::new(cmd, arg);

            dispatch_ioctl!(match raw_ioctl {
                cmd @ SetControlTty => {
                    let should_steal_tty = cmd.get() == 1;
                    Some(Self::SetControlTty { should_steal_tty })
                }
                cmd @ GetEventBits => {
                    let event_type = cmd.discriminant();
                    let buffer_length = cmd.base_ioctl().cmd.size();
                    Some(Self::GetEventBits {
                        event_type,
                        buffer_length,
                    })
                }
                _ => None,
            })
        }
    }

    #[ktest]
    fn invalid() {
        let action = Action::parse(0xFFFF, 1234);
        assert!(action.is_none());

        let action = Action::parse(0x540F, 4567);
        assert!(action.is_none());

        let action = Action::parse(0x8000540E, 1234);
        assert!(action.is_none());

        let action = Action::parse(0x0001540E, 4567);
        assert!(action.is_none());

        let action = Action::parse(0x40004520, 1234);
        assert!(action.is_none());

        let action = Action::parse(0x4000451F, 4567);
        assert!(action.is_none());

        let action = Action::parse(0x40004540, 4567);
        assert!(action.is_none());
    }

    #[ktest]
    fn tiocsctty() {
        let action = Action::parse(0x540E, 0);
        assert_eq!(
            action,
            Some(Action::SetControlTty {
                should_steal_tty: false
            })
        );

        let action = Action::parse(0x540E, 1);
        assert_eq!(
            action,
            Some(Action::SetControlTty {
                should_steal_tty: true
            })
        );

        let action = Action::parse(0x540E, 1 << 63);
        assert_eq!(
            action,
            Some(Action::SetControlTty {
                should_steal_tty: false
            })
        );
    }

    #[ktest]
    fn eviocgbit() {
        let action = Action::parse(0x80004520, 0);
        assert_eq!(
            action,
            Some(Action::GetEventBits {
                event_type: 0,
                buffer_length: 0
            })
        );

        let action = Action::parse(0x80104528, 1234);
        assert_eq!(
            action,
            Some(Action::GetEventBits {
                event_type: 8,
                buffer_length: 16
            })
        );

        let action = Action::parse(0xBFFF453F, 4567);
        assert_eq!(
            action,
            Some(Action::GetEventBits {
                event_type: 0x1F,
                buffer_length: 0x3FFF
            })
        );
    }
}
