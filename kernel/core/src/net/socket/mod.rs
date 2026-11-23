use core::fmt::Display;

use options::SocketOption;
use util::{MessageHeader, RecvFlags, RecvOutput, SendFlags, SockShutdownCmd, SocketAddr};

use crate::{
    fs::{
        file::{
            AccessMode, CreationFlags, FileCommon, FileLike, SettableStatusFlags, StatusFlags,
            file_table::FdFlags,
        },
        pseudofs::SockFs,
    },
    net::socket::util::ioctl::socket_ioctl,
    prelude::*,
    util::{MultiRead, MultiWrite, ioctl::RawIoctl},
};

pub(crate) mod ip;
pub(crate) mod netlink;
pub(crate) mod options;
pub(crate) mod unix;
pub(crate) mod util;
pub(crate) mod vsock;

mod private {
    use core::time::Duration;

    use crate::{events::IoEvents, prelude::*, process::signal::Pollable, util::ioctl::RawIoctl};

    pub(crate) trait SocketPrivate: Pollable {
        fn is_nonblocking(&self) -> bool;

        #[track_caller]
        fn block_on<F, R>(
            &self,
            events: IoEvents,
            timeout: Option<Duration>,
            mut try_op: F,
        ) -> Result<R>
        where
            Self: Sized,
            F: FnMut() -> Result<R>,
        {
            if self.is_nonblocking() {
                try_op()
            } else {
                self.wait_events(events, timeout.as_ref(), try_op)
                    .map_err(|err| match err.error() {
                        Errno::ETIME => {
                            errno!(EAGAIN, "the socket timeout expired")
                        }
                        _ => err,
                    })
            }
        }

        fn protocol_ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
            Err(errno!(ENOTTY, "the socket ioctl command is unknown"))
        }
    }
}

pub(crate) trait Socket: private::SocketPrivate + Send + Sync {
    fn bind(&self, _socket_addr: SocketAddr) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "bind() is not supported"))
    }

    fn connect(&self, _socket_addr: SocketAddr) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "connect() is not supported"))
    }

    fn listen(&self, _backlog: usize) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "listen() is not supported"))
    }

    fn accept(&self, _is_nonblocking: bool) -> Result<(Arc<dyn FileLike>, SocketAddr)> {
        Err(errno!(EOPNOTSUPP, "accept() is not supported"))
    }

    fn shutdown(&self, _cmd: SockShutdownCmd) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "shutdown() is not supported"))
    }

    fn addr(&self) -> Result<SocketAddr> {
        Err(errno!(EOPNOTSUPP, "getsockname() is not supported"))
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        Err(errno!(EOPNOTSUPP, "getpeername() is not supported"))
    }

    fn get_option(&self, _option: &mut dyn SocketOption) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "getsockopt() is not supported"))
    }

    fn set_option(&self, _option: &dyn SocketOption) -> Result<()> {
        Err(errno!(EOPNOTSUPP, "setsockopt() is not supported"))
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendFlags,
    ) -> Result<usize>;

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, MessageHeader)>;

    fn common(&self) -> &FileCommon;
}

impl<T: Socket + 'static> FileLike for T {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !writer.has_avail() {
            return Ok(0);
        }

        self.recvmsg(writer, RecvFlags::empty())
            .map(|(output, _)| output.len())
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.sendmsg(
            reader,
            MessageHeader::new(None, Vec::new()),
            SendFlags::empty(),
        )
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        socket_ioctl(self, raw_ioctl)
    }

    fn settable_status_flags(&self) -> SettableStatusFlags {
        SettableStatusFlags::minimal().with_o_async()
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        Some(self)
    }

    fn common(&self) -> &FileCommon {
        Socket::common(self)
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
            ino: u64,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                writeln!(f, "mnt_id:\t{}", SockFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", self.ino)
            }
        }

        let mut flags = self.common().status_flags().bits() | self.common().access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        Box::new(FdInfo {
            flags,
            ino: self.common().path().inode().ino(),
        })
    }
}

fn new_socket_common(is_nonblocking: bool) -> FileCommon {
    let status_flags = if is_nonblocking {
        StatusFlags::O_NONBLOCK
    } else {
        StatusFlags::empty()
    };
    FileCommon::new(SockFs::new_path(), AccessMode::O_RDWR, status_flags)
}
