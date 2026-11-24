use super::SyscallReturn;
use crate::{
    dispatch_ioctl,
    fs::file::{
        FileLike,
        file_table::{FdFlags, RawFileDesc, WithFileTable, get_file_fast},
    },
    prelude::*,
    process::posix_thread::FileTableRefMut,
    util::ioctl::RawIoctl,
};

pub(super) fn sys_ioctl(
    raw_fd: RawFileDesc,
    cmd: u32,
    arg: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let raw_ioctl = RawIoctl::new(cmd, arg);
    debug!("raw_fd = {}, raw_ioctl = {:#x?}", raw_fd, raw_ioctl,);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();

    if let Some(res) = handle_fd_ioctl(&mut file_table, raw_fd, raw_ioctl) {
        res?;
        return Ok(SyscallReturn::Return(0));
    }

    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

    let res = if let Some(res) = handle_file_ioctl(&**file, raw_ioctl) {
        res?;
        0
    } else {
        let file_owned = file.into_owned();

        drop(file_table);
        file_owned.ioctl(raw_ioctl)?
    };

    Ok(SyscallReturn::Return(res as _))
}

mod ioctl_defs {
    use crate::{
        ioc,
        util::ioctl::{InData, NoData},
    };

    pub(super) type SetNonBlocking    = ioc!(FIONBIO,  0x5421, InData<i32>);
    pub(super) type SetAsync          = ioc!(FIOASYNC, 0x5452, InData<i32>);

    pub(super) type SetNotCloseOnExec = ioc!(FIONCLEX, 0x5450, NoData);
    pub(super) type SetCloseOnExec    = ioc!(FIOCLEX,  0x5451, NoData);
}

fn handle_fd_ioctl(
    file_table: &mut FileTableRefMut,
    raw_fd: RawFileDesc,
    raw_ioctl: RawIoctl,
) -> Option<Result<()>> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        SetNotCloseOnExec => {
            Some(file_table.read_with(|inner| {
                let entry = inner.get_entry(raw_fd.try_into()?)?;

                entry.set_flags(entry.flags() - FdFlags::CLOEXEC);
                Ok(())
            }))
        }
        SetCloseOnExec => {
            Some(file_table.read_with(|inner| {
                let entry = inner.get_entry(raw_fd.try_into()?)?;

                entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                Ok(())
            }))
        }
        _ => None,
    })
}

fn handle_file_ioctl(file: &dyn FileLike, raw_ioctl: RawIoctl) -> Option<Result<()>> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ SetNonBlocking => {
            let handler = || {
                let is_nonblocking = cmd.read()? != 0;

                file.update_status_nonblock(is_nonblocking);
                Ok(())
            };
            Some(handler())
        }
        cmd @ SetAsync => {
            let handler = || {
                let is_async = cmd.read()? != 0;

                file.update_status_async(is_async)
            };
            Some(handler())
        }
        _ => None,
    })
}
