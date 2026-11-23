use crate::{
    prelude::*,
    process::{Gid, Uid},
};

mod ipc_ids;
mod ipc_ns;
pub(crate) mod semaphore;

pub(crate) use ipc_ids::IpcId;
pub(crate) use ipc_ns::IpcNamespace;

pub(crate) type IpcKey = i32;

const IPC_PRIVATE: IpcKey = 0;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct IpcFlags: u32 {
        const IPC_CREAT  = 1 << 9;
        const IPC_EXCL   = 1 << 10;
        const IPC_NOWAIT = 1 << 11;
        const SEM_UNDO   = 1 << 12;
    }
}

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum IpcControlCmd {
    IPC_RMID = 0,
    IPC_STAT = 2,

    SEM_GETPID = 11,
    SEM_GETVAL = 12,
    SEM_GETNCNT = 14,
    SEM_GETZCNT = 15,
    SEM_SETVAL = 16,
}

#[derive(Debug)]
pub(crate) struct IpcPermission {
    key: IpcKey,
    uid: Uid,
    gid: Gid,
    cuid: Uid,
    cguid: Gid,
    mode: u16,
}

impl IpcPermission {
    pub(crate) fn key(&self) -> IpcKey {
        self.key
    }

    pub(crate) fn uid(&self) -> Uid {
        self.uid
    }

    pub(crate) fn gid(&self) -> Gid {
        self.gid
    }

    pub(crate) fn cuid(&self) -> Uid {
        self.cuid
    }

    pub(crate) fn cguid(&self) -> Gid {
        self.cguid
    }

    pub(crate) fn mode(&self) -> u16 {
        self.mode
    }

    pub(self) fn new_sem_perm(key: IpcKey, uid: Uid, gid: Gid, mode: u16) -> Self {
        Self {
            key,
            uid,
            gid,
            cuid: uid,
            cguid: gid,
            mode,
        }
    }
}
