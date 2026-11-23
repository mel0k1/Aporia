#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

mod attr;
mod dirent;
mod error;
mod header;
mod ids;
mod operation;
pub mod ops;
mod status;

pub use self::{
    attr::{Attr, EntryReply},
    dirent::{DirOffset, Dirent, DirentType, FuseDirEntry},
    error::{FuseError, FuseResult},
    header::{ReplyHeader, ReqHeader},
    ids::{FuseFileHandle, FuseGeneration, FuseNodeId, FuseUnique, LookupCount},
    operation::{FuseOpcode, FuseOperation, ReplyExpectation},
    ops::{
        create::{CreateOperation, CreateReq},
        forget::{ForgetOperation, ForgetReq},
        fsync::{FsyncFlags, FsyncOperation, FsyncReq, FsyncdirOperation},
        getattr::{FuseAttrReply, GetattrFlags, GetattrOperation, GetattrReq},
        init::{FuseInitFlags, FuseInitFlags2, InitOperation, InitReply, InitReq},
        link::{LinkOperation, LinkReq},
        lookup::LookupOperation,
        lseek::{LseekOperation, LseekReply, LseekReq},
        mkdir::{MkdirOperation, MkdirReq},
        mknod::{MknodOperation, MknodReq},
        open::{FuseOpenFlags, OpenOperation, OpenReply, OpenReq, OpendirOperation},
        read::{ReadOperation, ReadReq},
        readdir::ReaddirOperation,
        readlink::{MAX_READLINK_LEN, ReadlinkOperation},
        release::{ReleaseFlags, ReleaseKind, ReleaseOperation, ReleaseReq},
        rename::{RenameOperation, RenameReq},
        rmdir::RmdirOperation,
        setattr::{SetattrOperation, SetattrReq, SetattrValid},
        statfs::{Kstatfs, StatfsOperation, StatfsReply},
        symlink::SymlinkOperation,
        unlink::UnlinkOperation,
        write::{WriteFlags, WriteOperation, WriteReply, WriteReq},
    },
    status::{FuseCompleteFn, FuseCompletion, FuseStatus},
};

pub const FUSE_ROOT_ID: FuseNodeId = FuseNodeId::new(1);

pub const FUSE_KERNEL_VERSION: u32 = 7;

pub const FUSE_KERNEL_MINOR_VERSION: u32 = 38;

pub const MIN_MAX_WRITE: u32 = 4096;
