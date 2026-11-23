use alloc::{
    borrow::ToOwned,
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::{
    any::Any,
    fmt::Debug,
    sync::atomic::{AtomicU64, Ordering},
};

use bitflags::bitflags;
use ostd::mm::{VmReader, VmWriter};

use super::{Error, Result, SysAttrSet, SysStr};

pub const MAX_ATTR_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SysNodeType {
    Branch,
    Leaf,
    Symlink,
}

#[expect(clippy::type_complexity)]
pub trait SysBranchNode: SysNode {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>));

    fn visit_children_with(
        &self,
        min_id: u64,
        f: &mut dyn for<'a> FnMut(&'a Arc<dyn SysObj>) -> Option<()>,
    );

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>>;

    fn children(&self) -> Vec<Arc<dyn SysObj>> {
        let mut children: Vec<Arc<dyn SysObj>> = Vec::new();
        self.visit_children_with(0, &mut |child_arc| {
            children.push(child_arc.clone());
            Some(())
        });
        children
    }

    fn count_children(&self) -> usize {
        let mut count = 0;
        self.visit_children_with(0, &mut |_| {
            count += 1;
            Some(())
        });
        count
    }

    fn create_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }

    fn remove_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }
}

pub trait SysNode: SysObj {
    fn node_attrs(&self) -> Arc<SysAttrSet>;

    fn is_attr_absent(&self, _name: &str) -> bool;

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize>;

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize>;

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize>;

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize>;

    fn show_attr(&self, name: &str) -> Result<String> {
        let mut buf: Vec<u8> = vec![0; MAX_ATTR_SIZE];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        let read_len = self.read_attr(name, &mut writer)?;
        let attr_val =
            String::from_utf8(buf[..read_len].to_vec()).map_err(|_| Error::InvalidOperation)?;
        Ok(attr_val)
    }

    fn store_attr(&self, name: &str, new_val: &str) -> Result<usize> {
        let mut reader = VmReader::from(new_val.as_bytes()).to_fallible();
        self.write_attr(name, &mut reader)
    }

    fn perms(&self) -> SysPerms;
}

pub trait SysSymlink: SysObj {
    fn target_path(&self) -> &str;
}

pub trait SysObj: Any + Send + Sync + Debug + 'static {
    fn as_any(&self) -> &dyn Any;

    fn cast_to_symlink(&self) -> Option<Arc<dyn SysSymlink>> {
        None
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        None
    }

    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        None
    }

    fn id(&self) -> &SysNodeId;

    fn type_(&self) -> SysNodeType;

    fn name(&self) -> &SysStr;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    fn path(&self) -> SysStr {
        if self.is_root() {
            return SysStr::from("/");
        }

        let Some(parent) = self.parent() else {
            return self.name().clone();
        };

        let parent_path_with_slash = {
            let mut parent_path = parent.path().as_ref().to_owned();
            if !parent.is_root() {
                parent_path.push('/');
            }

            parent_path
        };

        SysStr::from(parent_path_with_slash + self.name())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SysNodeId(u64);

impl SysNodeId {
    pub fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);

        let next_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        assert!(next_id <= u64::MAX / 2);

        Self(next_id)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for SysNodeId {
    fn default() -> Self {
        Self::new()
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub struct SysPerms: u16 {
        const OWNER_R = 0o0400;
        const OWNER_W = 0o0200;
        const OWNER_X = 0o0100;
        const GROUP_R = 0o0040;
        const GROUP_W = 0o0020;
        const GROUP_X = 0o0010;
        const OTHER_R = 0o0004;
        const OTHER_W = 0o0002;
        const OTHER_X = 0o0001;

        const ALL_R   = 0o0444;
        const ALL_W   = 0o0222;
        const ALL_X   = 0o0111;
        const ALL_RW  = 0o0666;
        const ALL_RX  = 0o0555;
        const ALL_RWX = 0o0777;
    }
}

impl SysPerms {
    pub const DEFAULT_RO_PERMS: Self = Self::ALL_RX;

    pub const DEFAULT_RW_PERMS: Self = Self::ALL_RX.union(Self::OWNER_W);

    pub const DEFAULT_RO_ATTR_PERMS: Self = Self::ALL_R;

    pub const DEFAULT_RW_ATTR_PERMS: Self = Self::ALL_R.union(Self::OWNER_W);

    pub fn can_read(&self) -> bool {
        self.intersects(Self::ALL_R)
    }

    pub fn can_write(&self) -> bool {
        self.intersects(Self::ALL_W)
    }
}
