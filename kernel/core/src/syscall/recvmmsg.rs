use ostd::mm::VmIo;

use super::{SyscallReturn, recvmsg::recv_one_message, sendmmsg::CMmsgHdr};
use crate::{fs::file::file_table::RawFileDesc, net::socket::util::RecvFlags, prelude::*};

pub(super) fn sys_recvmmsg(
    sockfd: RawFileDesc,
    mmsghdrs_addr: Vaddr,
    count: u32,
    flags: i32,
    timeout_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = RecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, mmsghdrs = {:#x}, count = {}, flags = {:?}, timeout_ptr = {:#x}",
        sockfd, mmsghdrs_addr, count, flags, timeout_ptr
    );

    if timeout_ptr != 0 {
        warn!("recvmmsg timeout is not supported");
    }

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.read();
        file_table_locked.get_file(sockfd.try_into()?)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let user_space = ctx.user_space();

    let mut received_msgs = 0;
    for i in 0..count {
        let addr = mmsghdrs_addr + size_of::<CMmsgHdr>() * i as usize;
        let mut mmsghdr = user_space.read_val::<CMmsgHdr>(addr)?;

        let received_bytes =
            match recv_one_message(socket, &mut mmsghdr.msg_hdr, &user_space, flags) {
                Ok(received_bytes) => received_bytes,
                Err(e) => {
                    if received_msgs == 0 {
                        return Err(e);
                    }
                    break;
                }
            };

        mmsghdr.msg_len = received_bytes as u32;
        user_space.write_val(addr, &mmsghdr)?;

        received_msgs += 1;
    }

    Ok(SyscallReturn::Return(received_msgs as _))
}
