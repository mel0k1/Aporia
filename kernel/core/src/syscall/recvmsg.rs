use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::file_table::RawFileDesc,
    net::socket::{Socket, util::RecvFlags},
    prelude::*,
    util::net::CUserMsgHdr,
};

pub(super) fn sys_recvmsg(
    sockfd: RawFileDesc,
    user_msghdr_ptr: Vaddr,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let mut c_user_msghdr: CUserMsgHdr = user_space.read_val(user_msghdr_ptr)?;
    let flags = RecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.read();
        file_table_locked.get_file(sockfd.try_into()?)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let received_bytes = recv_one_message(socket, &mut c_user_msghdr, &user_space, flags)?;

    user_space.write_val(user_msghdr_ptr, &c_user_msghdr)?;

    Ok(SyscallReturn::Return(received_bytes as _))
}

pub(super) fn recv_one_message(
    socket: &dyn Socket,
    c_user_msghdr: &mut CUserMsgHdr,
    user_space: &CurrentUserSpace,
    flags: RecvFlags,
) -> Result<usize> {
    let (output, message_header) = {
        let mut io_vec_writer = c_user_msghdr.copy_writer_array_from_user(user_space)?;
        socket
            .recvmsg(&mut io_vec_writer, flags)
            .map_err(|err| match err.error() {
                Errno::EINTR => errno!(ERESTARTSYS),
                _ => err,
            })?
    };

    let addr = message_header.addr();
    c_user_msghdr.msg_namelen = c_user_msghdr.write_socket_addr_to_user(addr)?;

    let control_messages = message_header.control_messages();
    let (control_len, control_flags) =
        c_user_msghdr.write_control_messages_to_user(control_messages, user_space)?;
    c_user_msghdr.msg_controllen = control_len as _;
    c_user_msghdr.msg_flags = (output.flags() | control_flags).bits().cast_unsigned();

    Ok(output.len())
}
