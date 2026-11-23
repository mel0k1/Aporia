#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod attr;
mod node;
#[cfg(ktest)]
mod test;
mod tree;
mod utils;

use alloc::{borrow::Cow, sync::Arc};

use aster_util::printer::VmPrinterError;
use component::{ComponentInitError, init_component};
use spin::Once;

pub use self::{
    attr::{SysAttr, SysAttrSet, SysAttrSetBuilder},
    node::{
        MAX_ATTR_SIZE, SysBranchNode, SysNode, SysNodeId, SysNodeType, SysObj, SysPerms, SysSymlink,
    },
    tree::SysTree,
    utils::{
        _InheritSysBranchNode, _InheritSysLeafNode, _InheritSysSymlinkNode,
        AttrLessBranchNodeFields, BranchNodeFields, EmptyNode, NormalNodeFields, ObjFields,
        SymlinkNodeFields, is_valid_name, relative_path,
    },
};
use crate::tree::RootNode;

static SINGLETON: Once<Arc<SysTree<RootNode>>> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
    Ok(())
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    SINGLETON.call_once(|| Arc::new(SysTree::new()));
}

pub fn primary_tree() -> &'static Arc<SysTree<RootNode>> {
    SINGLETON.get().expect("SysTree not initialized")
}

pub type SysStr = Cow<'static, str>;

pub type Result<T, E = Error> = core::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    NotFound,
    InvalidOperation,
    InvalidName,
    ResourceUnavailable,
    MemoryBudgetExhausted,
    AttributeError,
    PermissionDenied,
    InternalError(&'static str),
    AlreadyExists,
    Overflow,
    PageFault,
    IsDead,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Error::NotFound => write!(f, "attempted to access a non-existent systree item"),
            Error::InvalidOperation => write!(f, "invalid operation occurred"),
            Error::InvalidName => write!(f, "invalid name"),
            Error::ResourceUnavailable => write!(f, "resource is unavailable"),
            Error::MemoryBudgetExhausted => write!(f, "attribute memory budget is exhausted"),
            Error::AttributeError => write!(f, "attribute error"),
            Error::PermissionDenied => write!(f, "permission denied for operation"),
            Error::InternalError(msg) => write!(f, "internal error: {}", msg),
            Error::AlreadyExists => write!(f, "the systree item already exists"),
            Error::Overflow => write!(f, "numerical overflow occurred"),
            Error::PageFault => write!(f, "page fault occurred during memory access"),
            Error::IsDead => write!(f, "the current systree item is dead"),
        }
    }
}

impl From<VmPrinterError> for Error {
    fn from(value: VmPrinterError) -> Self {
        match value {
            VmPrinterError::PageFault => Error::PageFault,
        }
    }
}
