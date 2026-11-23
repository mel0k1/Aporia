use alloc::{borrow::Cow, format, vec};
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Error as SysTreeError, NormalNodeFields, ObjFields, Result as SysTreeResult,
    SymlinkNodeFields, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysNode, SysNodeId,
    SysNodeType, SysObj, SysPerms, SysStr, inherit_sys_branch_node, inherit_sys_leaf_node,
    inherit_sys_symlink_node, init_for_ktest,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::prelude::ktest;

use crate::{
    fs::{
        file::{InodeType, StatusFlags, mkmod},
        sysfs::{fs::SysFs, inode::SysFsInode},
        utils::{DirentVisitor, systree_inode::SysTreeInodeTy},
        vfs::{file_system::FileSystem, path::Dentry},
    },
    prelude::*,
    time::clocks::init_for_ktest as time_init_for_ktest,
};

#[derive(Debug)]
struct MockLeafNode {
    fields: NormalNodeFields<Self>,
    data: RwLock<BTreeMap<String, String>>,
}

impl MockLeafNode {
    fn new(name: SysStr, read_attrs: &[&str], write_attrs: &[&str]) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        let mut data = BTreeMap::new();
        for &attr_name in read_attrs {
            builder.add(
                Cow::Owned(attr_name.to_string()),
                SysPerms::DEFAULT_RO_ATTR_PERMS,
            );
            data.insert(attr_name.to_string(), format!("val_{}", attr_name));
        }
        for &attr_name in write_attrs {
            builder.add(
                Cow::Owned(attr_name.to_string()),
                SysPerms::DEFAULT_RW_ATTR_PERMS,
            );
            data.insert(attr_name.to_string(), format!("val_{}", attr_name));
        }

        let attrs = builder.build().expect("Failed to build attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = NormalNodeFields::new(name, attrs, weak_self.clone());
            MockLeafNode {
                fields,
                data: RwLock::new(data),
            }
        })
    }
}

inherit_sys_leaf_node!(MockLeafNode, fields, {
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_read() {
            return Err(SysTreeError::PermissionDenied);
        }
        let data = self.data.read();
        let value = data.get(name).ok_or(SysTreeError::NotFound)?;

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", value)?;

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_write() {
            return Err(SysTreeError::PermissionDenied);
        }

        let mut buffer = [0u8; 1024];
        let mut writer = VmWriter::from(&mut buffer[..]);
        let read_len = reader
            .read_fallible(&mut writer)
            .map_err(|_| SysTreeError::PageFault)?;

        let new_value = String::from_utf8_lossy(&buffer[..read_len]).to_string();

        let mut data = self.data.write();
        data.insert(name.to_string(), new_value);

        Ok(read_len)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct MockBranchNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

impl MockBranchNode {
    fn new(name: &str) -> Arc<Self> {
        let name_owned: SysStr = name.to_string().into();

        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            Cow::Borrowed("branch_attr"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        let attrs = builder
            .build()
            .expect("Failed to build branch attribute set");

        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name_owned, attrs, weak_self.clone());
            MockBranchNode { fields }
        })
    }

    fn add_child(&self, child: Arc<dyn SysObj>) {
        self.fields.add_child(child).unwrap();
    }
}

inherit_sys_branch_node!(MockBranchNode, fields, {
    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_read() {
            return Err(SysTreeError::PermissionDenied);
        }
        let value = match name {
            "branch_attr" => "branch_value",
            _ => return Err(SysTreeError::NotFound),
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", value)?;

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        let attr = self
            .fields
            .attr_set()
            .get(name)
            .ok_or(SysTreeError::NotFound)?;
        if !attr.perms().can_write() {
            return Err(SysTreeError::PermissionDenied);
        }
        Err(SysTreeError::AttributeError)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});

#[derive(Debug)]
struct MockSymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl MockSymlinkNode {
    fn new(name: SysStr, target: &str) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name, target.to_string(), weak_self.clone());
            MockSymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(MockSymlinkNode, fields);

#[derive(Debug)]
struct MockMutableAttrNode {
    fields: ObjFields<Self>,
    attrs: RwLock<Arc<SysAttrSet>>,
}

impl MockMutableAttrNode {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            fields: ObjFields::new("device".into(), this.clone()),
            attrs: RwLock::new(SysAttrSet::empty().clone()),
        })
    }
}

#[inherit_methods(from = "self.fields")]
impl SysObj for MockMutableAttrNode {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.fields.weak_self().upgrade().map(|node| node as _)
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Leaf
    }

    fn id(&self) -> &SysNodeId;
    fn name(&self) -> &SysStr;
    fn init_parent(&self, parent: Weak<dyn SysBranchNode>);
    fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;
}

impl SysNode for MockMutableAttrNode {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.attrs.read().clone()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> SysTreeResult<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn read_attr_at(
        &self,
        _name: &str,
        _offset: usize,
        _writer: &mut VmWriter,
    ) -> SysTreeResult<usize> {
        Err(SysTreeError::AttributeError)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> SysTreeResult<usize> {
        Err(SysTreeError::AttributeError)
    }

    fn write_attr_at(
        &self,
        name: &str,
        _offset: usize,
        reader: &mut VmReader,
    ) -> SysTreeResult<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

fn init_sysfs(root_node: Arc<dyn SysBranchNode>) -> Arc<Dentry> {
    time_init_for_ktest();
    init_for_ktest();
    let sysfs = SysFs::new_for_ktest();
    Dentry::new_root(SysFsInode::new_root(root_node, &sysfs.sb()))
}

fn create_mock_systree() -> Arc<MockBranchNode> {
    let root = MockBranchNode::new("root");
    let branch1 = MockBranchNode::new("branch1");
    let leaf1 = MockLeafNode::new("leaf1".into(), &["r_attr1"], &["rw_attr1"]);
    let leaf2 = MockLeafNode::new("leaf2".into(), &["r_attr2"], &[]);
    let symlink1 = MockSymlinkNode::new("link1".into(), "../branch1/leaf1");

    branch1.add_child(leaf1);
    root.add_child(branch1);
    root.add_child(leaf2);
    root.add_child(symlink1);
    root
}

#[ktest]
fn root_lookup() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();

    assert_eq!(root_inode.type_(), InodeType::Dir);

    let branch1_inode = root_inode.lookup("branch1").expect("Lookup branch1 failed");
    assert_eq!(branch1_inode.type_(), InodeType::Dir);

    let leaf2_inode = root_inode.lookup("leaf2").expect("Lookup leaf2 failed");
    assert_eq!(leaf2_inode.type_(), InodeType::Dir);

    let link1_inode = root_inode.lookup("link1").expect("Lookup link1 failed");
    assert_eq!(link1_inode.type_(), InodeType::SymLink);

    let result = root_inode.lookup("nonexistent");
    assert!(result.is_err());

    let self_inode = root_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), root_inode.ino());

    let parent_inode = root_inode.lookup("..").expect("Lookup .. failed");
    assert_eq!(parent_inode.ino(), root_inode.ino());
}

#[ktest]
fn branch_lookup() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let branch1_inode = root_inode.lookup("branch1").unwrap();

    let leaf1_inode = branch1_inode.lookup("leaf1").expect("Lookup leaf1 failed");
    assert_eq!(leaf1_inode.type_(), InodeType::Dir);

    let attr_inode = branch1_inode
        .lookup("branch_attr")
        .expect("Lookup branch_attr failed");
    assert_eq!(attr_inode.type_(), InodeType::File);

    let result = branch1_inode.lookup("nonexistent_leaf");
    assert!(result.is_err());

    let self_inode = branch1_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), branch1_inode.ino());

    let parent_inode = branch1_inode.lookup("..").expect("Lookup .. failed");
    assert_eq!(parent_inode.ino(), root_inode.ino());
}

#[ktest]
fn leaf_lookup() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let leaf1_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();

    assert_eq!(leaf1_inode.type_(), InodeType::Dir);

    let r_attr_inode = leaf1_inode
        .lookup("r_attr1")
        .expect("Lookup r_attr1 failed");
    assert_eq!(r_attr_inode.type_(), InodeType::File);

    let rw_attr_inode = leaf1_inode
        .lookup("rw_attr1")
        .expect("Lookup rw_attr1 failed");
    assert_eq!(rw_attr_inode.type_(), InodeType::File);

    let result = leaf1_inode.lookup("nonexistent_attr");
    assert!(result.is_err());

    let self_inode = leaf1_inode.lookup(".").expect("Lookup . failed");
    assert_eq!(self_inode.ino(), leaf1_inode.ino());
}

#[ktest]
fn read_attr() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap();

    let mut buf = [0u8; 64];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let bytes_read = r_attr_inode
        .read_at(0, &mut writer, StatusFlags::empty())
        .expect("read_at failed");

    assert!(bytes_read > 0);
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, "val_r_attr1");

    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let result = leaf1_dir_inode.read_at(0, &mut writer, StatusFlags::empty());
    assert!(result.is_err());
}

#[ktest]
fn write_attr() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    let rw_attr_inode = leaf1_dir_inode.lookup("rw_attr1").unwrap();
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap();

    let new_val = "new_value";
    let mut reader = VmReader::from(new_val.as_bytes()).to_fallible();
    let bytes_written = rw_attr_inode
        .write_at(0, &mut reader, StatusFlags::empty())
        .expect("write_at failed");
    assert_eq!(bytes_written, new_val.len());

    let mut buf = [0u8; 64];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let bytes_read = rw_attr_inode
        .read_at(0, &mut writer, StatusFlags::empty())
        .expect("read_at failed");
    let content = core::str::from_utf8(&buf[..bytes_read]).unwrap();
    assert_eq!(content, new_val);

    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = r_attr_inode.write_at(0, &mut reader, StatusFlags::empty());
    assert!(result.is_err());

    let mut reader = VmReader::from("attempt_write".as_bytes()).to_fallible();
    let result = leaf1_dir_inode.write_at(0, &mut reader, StatusFlags::empty());
    assert!(result.is_err());
}

#[ktest]
fn read_link() {
    use crate::fs::vfs::inode::SymbolicLink;

    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let link1_inode = root_inode.lookup("link1").unwrap();

    let target = link1_inode.read_link().expect("read_link failed");
    assert!(matches!(
        target,
        SymbolicLink::Plain(s) if s == "../branch1/leaf1"
    ));

    let branch1_inode = root_inode.lookup("branch1").unwrap();
    let result = branch1_inode.read_link();
    assert!(result.is_err());
}

struct TestDirentVisitor {
    entries: Vec<(String, u64, InodeType)>,
}

impl DirentVisitor for TestDirentVisitor {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, _next_offset: usize) -> Result<()> {
        self.entries.push((name.to_string(), ino, type_));
        Ok(())
    }
}

#[ktest]
fn readdir_leaf() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let leaf1_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    let mut visitor = TestDirentVisitor { entries: vec![] };

    let mut offset = 0;
    loop {
        let result = leaf1_inode.readdir_at(offset, &mut visitor);
        match result {
            Ok(next_offset) => {
                if next_offset == offset || next_offset == 0 {
                    break;
                }
                offset = next_offset;
            }
            Err(e) => {
                panic!("readdir_at failed unexpectedly: {:?}", e);
            }
        }
    }

    let mut names: Vec<_> = visitor.entries.iter().map(|(n, _, _)| n.clone()).collect();
    names.sort();

    assert!(names.iter().any(|n| n == "."));
    assert!(names.iter().any(|n| n == ".."));
    assert!(names.iter().any(|n| n == "r_attr1"));
    assert!(names.iter().any(|n| n == "rw_attr1"));

    for (name, _, type_) in &visitor.entries {
        match name.as_str() {
            "." | ".." => assert_eq!(*type_, InodeType::Dir),
            "r_attr1" | "rw_attr1" => assert_eq!(*type_, InodeType::File),
            _ => panic!("Unexpected entry: {}", name),
        }
    }
}

#[ktest]
fn mode_permissions() {
    let root_dentry = init_sysfs(create_mock_systree());
    let root_inode = root_dentry.inode();
    let leaf1_dir_inode = root_inode
        .lookup("branch1")
        .unwrap()
        .lookup("leaf1")
        .unwrap();
    let r_attr_inode = leaf1_dir_inode.lookup("r_attr1").unwrap();
    let rw_attr_inode = leaf1_dir_inode.lookup("rw_attr1").unwrap();

    let r_mode = r_attr_inode.mode().unwrap();
    assert!(r_mode.contains(mkmod!(a+r)));
    assert!(!r_mode.contains(mkmod!(u+w)));

    let rw_mode = rw_attr_inode.mode().unwrap();
    assert!(rw_mode.contains(mkmod!(a+r)));
    assert!(rw_mode.contains(mkmod!(u+w)));

    let new_mode = mkmod!(u+rw);
    let rw_attr_dentry = Dentry::new_root(rw_attr_inode.clone());
    rw_attr_inode
        .set_mode(&rw_attr_dentry, new_mode)
        .expect("set_mode failed");
    assert_eq!(rw_attr_inode.mode().unwrap(), new_mode);

    let leaf1_mode = leaf1_dir_inode.mode().unwrap();
    assert!(leaf1_mode.contains(mkmod!(a+rx)));
}

#[ktest]
fn cached_child_lookup_observes_tree_changes() {
    let branch = MockBranchNode::new("root");
    let root_dentry = init_sysfs(branch.clone());
    let dir = root_dentry.as_dir_dentry_or_err().unwrap();

    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );

    branch.add_child(MockLeafNode::new("child".into(), &[], &[]));
    let leaf = dir.lookup_child("child").unwrap();
    assert_eq!(leaf.inode().type_(), InodeType::Dir);
    assert!(Arc::ptr_eq(&leaf, &dir.lookup_child("child").unwrap()));

    branch.fields.remove_child("child").unwrap();
    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );

    branch.add_child(MockBranchNode::new("child"));
    let child_dir = dir.lookup_child("child").unwrap();
    assert_ne!(child_dir.inode().ino(), leaf.inode().ino());

    branch.fields.remove_child("child").unwrap();
    branch.add_child(MockSymlinkNode::new("child".into(), "target"));
    let link = dir.lookup_child("child").unwrap();
    assert_eq!(link.inode().type_(), InodeType::SymLink);
    assert_ne!(link.inode().ino(), child_dir.inode().ino());
    assert!(Arc::ptr_eq(&link, &dir.lookup_child("child").unwrap()));

    branch.fields.remove_child("child").unwrap();
    assert_eq!(
        dir.lookup_child("child").unwrap_err().error(),
        Errno::ENOENT
    );
}

#[ktest]
fn cached_attr_lookup_observes_snapshot_changes() {
    let branch = MockBranchNode::new("root");
    let node = MockMutableAttrNode::new();
    branch.add_child(node.clone());
    let root_dentry = init_sysfs(branch);
    let leaf = root_dentry
        .as_dir_dentry_or_err()
        .unwrap()
        .lookup_child("device")
        .unwrap();
    let dir = leaf.as_dir_dentry_or_err().unwrap();

    assert_eq!(
        dir.lookup_child("status").unwrap_err().error(),
        Errno::ENOENT
    );

    let attrs = {
        let mut builder = SysAttrSetBuilder::new();
        builder.add("status".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    *node.attrs.write() = attrs.clone();
    let attr = dir.lookup_child("status").unwrap();
    assert_eq!(attr.inode().type_(), InodeType::File);
    assert!(Arc::ptr_eq(&attr, &dir.lookup_child("status").unwrap()));

    *node.attrs.write() = SysAttrSet::empty().clone();
    assert_eq!(
        dir.lookup_child("status").unwrap_err().error(),
        Errno::ENOENT
    );

    *node.attrs.write() = attrs;
    let restored = dir.lookup_child("status").unwrap();
    assert!(!Arc::ptr_eq(&attr, &restored));
    assert_eq!(restored.inode().type_(), InodeType::File);
}

#[ktest]
fn cached_attr_lookup_observes_permission_changes() {
    let branch = MockBranchNode::new("root");
    let node = MockMutableAttrNode::new();
    let attrs = {
        let mut builder = SysAttrSetBuilder::new();
        builder.add("status".into(), SysPerms::DEFAULT_RO_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    *node.attrs.write() = attrs;
    branch.add_child(node.clone());
    let root_dentry = init_sysfs(branch);
    let leaf = root_dentry
        .as_dir_dentry_or_err()
        .unwrap()
        .lookup_child("device")
        .unwrap();
    let dir = leaf.as_dir_dentry_or_err().unwrap();

    let attr = dir.lookup_child("status").unwrap();
    assert_eq!(attr.inode().mode().unwrap(), mkmod!(a+r));

    attr.inode().set_mode(&attr, mkmod!(u+r)).unwrap();
    let cached = dir.lookup_child("status").unwrap();
    assert!(Arc::ptr_eq(&attr, &cached));
    assert_eq!(cached.inode().mode().unwrap(), mkmod!(u+r));

    let old_attrs = node.node_attrs();
    let new_attrs = {
        let mut builder = SysAttrSetBuilder::from_set(&old_attrs);
        builder.remove("status");
        builder.add("status".into(), SysPerms::DEFAULT_RW_ATTR_PERMS);
        Arc::new(builder.build().unwrap())
    };
    assert_eq!(
        old_attrs.get("status").unwrap().id(),
        new_attrs.get("status").unwrap().id()
    );
    *node.attrs.write() = new_attrs;

    let updated = dir.lookup_child("status").unwrap();
    assert!(!Arc::ptr_eq(&attr, &updated));
    assert_eq!(updated.inode().mode().unwrap(), mkmod!(u+rw, a+r));
}
