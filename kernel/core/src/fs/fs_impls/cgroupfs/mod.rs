pub(crate) use cgroup_ns::CgroupNamespace;
pub(crate) use controller::cpu::{CpuStatKind, charge_cpu_time};
use fs::CgroupFsType;
pub(in crate::fs) use systree_node::CgroupSystem;
pub(crate) use systree_node::{CgroupMembership, CgroupNode, CgroupSysNode};

macro_rules! __log_prefix {
    () => {
        "cgroup: "
    };
}

mod cgroup_ns;
mod controller;
mod fs;
mod inode;
mod systree_node;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&CgroupFsType).unwrap();
}
