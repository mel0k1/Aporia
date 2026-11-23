use core::{
    str::FromStr,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_systree::{
    BranchNodeFields, Error, MAX_ATTR_SIZE, Result, SysAttrSetBuilder, SysBranchNode, SysObj,
    SysPerms, SysStr, inherit_sys_branch_node,
};
use aster_util::printer::VmPrinter;
use inherit_methods_macro::inherit_methods;
use ostd::sync::{RwMutexReadGuard, RwMutexWriteGuard};
use spin::Once;

use crate::{
    fs::cgroupfs::controller::{Controller, PidsPreCharge, SubCtrlSet, SubCtrlType},
    prelude::*,
    process::{Pid, Process, pid_table},
};

pub(crate) struct CgroupMembership {
    _private: (),
}

fn global_cgroup_membership() -> &'static RwMutex<CgroupMembership> {
    static CGROUP_MEMBERSHIP: RwMutex<CgroupMembership> =
        RwMutex::new(CgroupMembership { _private: () });
    &CGROUP_MEMBERSHIP
}

impl CgroupMembership {
    pub(crate) fn write_lock() -> RwMutexWriteGuard<'static, Self> {
        global_cgroup_membership().write()
    }

    pub(crate) fn read_lock() -> RwMutexReadGuard<'static, Self> {
        global_cgroup_membership().read()
    }

    pub(super) fn count_subtree_processes(&mut self, cgroup_node: &CgroupNode) -> u32 {
        let mut total: u32 = 0;
        let mut stack: Vec<Arc<dyn SysObj>> = vec![];

        total += cgroup_node
            .with_inner(|procs| procs.len() as u32)
            .unwrap_or(0);
        cgroup_node.visit_children_with(0, &mut |child| {
            stack.push(child.clone());
            Some(())
        });

        while let Some(node) = stack.pop() {
            let cgroup_node = Arc::downcast::<CgroupNode>(node).unwrap();

            total += cgroup_node
                .with_inner(|procs| procs.len() as u32)
                .unwrap_or(0);

            cgroup_node.visit_children_with(0, &mut |child| {
                stack.push(child.clone());
                Some(())
            });
        }

        total
    }

    pub(crate) fn move_process_to_node(
        &mut self,
        process: Arc<Process>,
        new_cgroup: &CgroupNode,
    ) -> Result<()> {
        let old_cgroup = if let Some(old_cgroup) = process.cgroup().get() {
            if new_cgroup.id() == old_cgroup.id() {
                return Ok(());
            }

            Some(old_cgroup.clone())
        } else {
            None
        };

        self.add_process_to_node(&process, new_cgroup)?;
        new_cgroup.controller.charge_pids();

        if let Some(old_cgroup) = old_cgroup {
            old_cgroup
                .with_inner_mut(|old_cgroup_processes| {
                    old_cgroup_processes.remove(&process.pid()).unwrap();
                    if old_cgroup_processes.is_empty() {
                        let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                        if old_count == 1 {
                            old_cgroup.propagate_sub_populated();
                        }
                    }
                })
                .unwrap();

            old_cgroup.controller.uncharge_pids();
        }

        Ok(())
    }

    pub(crate) fn move_forked_process_to_node(
        &self,
        process: Arc<Process>,
        new_cgroup: &CgroupNode,
        pids_charge: PidsPreCharge<'_>,
    ) {
        debug_assert!(process.cgroup().get().is_none());

        pids_charge.apply();
        self.add_process_to_node(&process, new_cgroup).unwrap();
    }

    fn add_process_to_node(&self, process: &Arc<Process>, new_cgroup: &CgroupNode) -> Result<()> {
        if !new_cgroup.controller.active_set().is_empty() {
            return Err(Error::ResourceUnavailable);
        }
        new_cgroup
            .with_inner_mut(|current_processes| {
                if current_processes.is_empty() {
                    let old_count = new_cgroup.populated_count.fetch_add(1, Ordering::Relaxed);
                    if old_count == 0 {
                        new_cgroup.propagate_add_populated();
                    }
                }

                current_processes.insert(process.pid(), Arc::downgrade(process));
                process.set_cgroup(Some(new_cgroup.fields.weak_self().upgrade().unwrap()));
            })
            .ok_or(Error::IsDead)?;

        Ok(())
    }

    pub(crate) fn move_process_to_root(&mut self, process: &Process) {
        let old_cgroup = if let Some(old_cgroup) = process.cgroup().get() {
            old_cgroup.clone()
        } else {
            return;
        };

        process.set_cgroup(None);

        old_cgroup
            .with_inner_mut(|old_cgroup_processes| {
                old_cgroup_processes.remove(&process.pid()).unwrap();
                if old_cgroup_processes.is_empty() {
                    let old_count = old_cgroup.populated_count.fetch_sub(1, Ordering::Relaxed);
                    if old_count == 1 {
                        old_cgroup.propagate_sub_populated();
                    }
                }
            })
            .unwrap();

        old_cgroup.controller.uncharge_pids();
    }
}

pub(in crate::fs) struct CgroupSystem {
    fields: BranchNodeFields<CgroupNode, Self>,
    controller: Controller,
}

impl Debug for CgroupSystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupSystem")
            .field("fields", &self.fields)
            .finish_non_exhaustive()
    }
}

pub(crate) struct CgroupNode {
    fields: BranchNodeFields<CgroupNode, Self>,
    controller: Controller,
    inner: RwMutex<Option<Inner>>,
    depth: usize,
    populated_count: AtomicUsize,
}

impl Debug for CgroupNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CgroupNode")
            .field("fields", &self.fields)
            .field("populated_count", &self.populated_count)
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Inner {
    processes: BTreeMap<Pid, Weak<Process>>,
}

#[inherit_methods(from = "self.fields")]
impl CgroupSystem {
    fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

#[inherit_methods(from = "self.fields")]
impl CgroupNode {
    fn add_child(&self, new_child: Arc<CgroupNode>) -> Result<()>;
}

impl CgroupSystem {
    pub(in crate::fs) fn singleton() -> &'static Arc<CgroupSystem> {
        static SINGLETON: Once<Arc<CgroupSystem>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let name = SysStr::from("cgroup");

        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.procs"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.subtree_control"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );

        Controller::init_attr_set(&mut builder, true);

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            CgroupSystem {
                fields,
                controller: Controller::new(None),
            }
        })
    }
}

impl CgroupSysNode for CgroupSystem {
    fn controller(&self) -> &Controller {
        &self.controller
    }
}

impl CgroupNode {
    pub(self) fn new(name: SysStr, depth: usize, parent_controller: &Controller) -> Arc<Self> {
        let mut builder = SysAttrSetBuilder::new();
        builder.add(
            SysStr::from("cgroup.controllers"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.events"),
            SysPerms::DEFAULT_RO_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.freeze"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.max.depth"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.procs"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.subtree_control"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );
        builder.add(
            SysStr::from("cgroup.threads"),
            SysPerms::DEFAULT_RW_ATTR_PERMS,
        );

        Controller::init_attr_set(&mut builder, false);

        let attrs = builder.build().expect("Failed to build attribute set");
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            CgroupNode {
                fields,
                controller: Controller::new(Some(parent_controller)),
                inner: RwMutex::new(Some(Inner::default())),
                depth,
                populated_count: AtomicUsize::new(0),
            }
        })
    }
}

impl CgroupSysNode for CgroupNode {
    fn controller(&self) -> &Controller {
        &self.controller
    }
}

impl CgroupNode {
    fn propagate_add_populated(&self) {
        if self.depth <= 1 {
            return;
        }

        let mut current_parent = Arc::downcast::<CgroupNode>(self.parent().unwrap()).unwrap();
        loop {
            let old_count = current_parent
                .populated_count
                .fetch_add(1, Ordering::Relaxed);
            if old_count > 0 {
                break;
            }

            if current_parent.depth == 1 {
                break;
            }

            current_parent = Arc::downcast::<CgroupNode>(current_parent.parent().unwrap()).unwrap();
        }
    }

    fn propagate_sub_populated(&self) {
        if self.depth <= 1 {
            return;
        }

        let mut current_parent = Arc::downcast::<CgroupNode>(self.parent().unwrap()).unwrap();
        loop {
            let old_count = current_parent
                .populated_count
                .fetch_sub(1, Ordering::Relaxed);
            if old_count != 1 {
                break;
            }

            if current_parent.depth == 1 {
                break;
            }

            current_parent = Arc::downcast::<CgroupNode>(current_parent.parent().unwrap()).unwrap();
        }
    }

    #[must_use]
    fn with_inner<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&BTreeMap<Pid, Weak<Process>>) -> R,
    {
        let inner = self.inner.read();
        let inner_ref = inner.as_ref()?;

        Some(op(&inner_ref.processes))
    }

    #[must_use]
    fn with_inner_mut<F, R>(&self, op: F) -> Option<R>
    where
        F: FnOnce(&mut BTreeMap<Pid, Weak<Process>>) -> R,
    {
        let mut inner = self.inner.write();
        let inner_ref = inner.as_mut()?;

        Some(op(&mut inner_ref.processes))
    }

    pub(super) fn mark_as_dead(&self) -> crate::prelude::Result<()> {
        let mut inner = self.inner.write();
        let Some(inner_ref) = inner.as_ref() else {
            return Err(errno!(ENOENT, "the cgroup node is already dead"));
        };

        if !inner_ref.processes.is_empty() {
            return Err(errno!(EBUSY, "the cgroup hierarchy still has processes"));
        }

        let children = self.fields.children_ref().read();
        if !children.is_empty() {
            return Err(errno!(
                ENOTEMPTY,
                "only an empty cgroup hierarchy can be removed"
            ));
        }

        *inner = None;

        Ok(())
    }
}

inherit_sys_branch_node!(CgroupSystem, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {}

    fn is_attr_absent(&self, name: &str) -> bool {
        if name.starts_with("cgroup.") {
            false
        } else {
            self.controller.is_attr_absent(name)
        }
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "cgroup.controllers" => {
                writeln!(printer, "{}", SubCtrlSet::all())?;
            }
            "cgroup.procs" => {
                let pid_table = pid_table::pid_table_mut();
                for process in pid_table.iter_processes() {
                    if process.cgroup().is_none() {
                        writeln!(printer, "{}", process.pid())?;
                    }
                }
            }
            "cgroup.subtree_control" => {
                let active_set = self.controller.active_set();
                writeln!(printer, "{}", active_set)?;
            }
            _ => return self.controller.read_attr_at(name, offset, writer),
        }

        Ok(printer.bytes_written())
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let pid = content
                    .to_str()
                    .ok()
                    .and_then(|string| string.trim().parse::<Pid>().ok())
                    .ok_or(Error::InvalidOperation)?;

                with_process_cgroup_locked(pid, |process, cgroup_membership| {
                    cgroup_membership.move_process_to_root(&process);
                    Ok(())
                })?;

                Ok(len)
            }
            "cgroup.subtree_control" => {
                let (activate_set, deactivate_set, len) = read_subtree_control_from_reader(reader)?;

                let mut cgroup_guard = CgroupMembership::write_lock();
                for ctrl_type in activate_set.iter_types() {
                    self.controller
                        .activate(ctrl_type, self, None, &mut cgroup_guard)?;
                }
                for ctrl_type in deactivate_set.iter_types() {
                    self.controller
                        .deactivate(ctrl_type, self, &mut cgroup_guard)?;
                }

                Ok(len)
            }
            _ => self.controller.write_attr(name, reader),
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let _cgroup_read_guard = CgroupMembership::read_lock();
        let new_child = CgroupNode::new(name.to_string().into(), 1, &self.controller);
        self.add_child(new_child.clone())?;
        Ok(new_child)
    }
});

inherit_sys_branch_node!(CgroupNode, fields, {
    fn is_attr_absent(&self, name: &str) -> bool {
        if name.starts_with("cgroup.") {
            false
        } else {
            self.controller.is_attr_absent(name)
        }
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        match name {
            "cgroup.controllers" => {
                let active_set = self
                    .cgroup_parent()
                    .ok_or(Error::IsDead)?
                    .controller()
                    .active_set();
                self.with_inner(|_| {
                    writeln!(printer, "{}", active_set)?;

                    Ok::<usize, Error>(printer.bytes_written())
                })
                .ok_or(Error::IsDead)?
            }
            "cgroup.events" => self
                .with_inner(|_| {
                    let res = if self.populated_count.load(Ordering::Relaxed) > 0 {
                        1
                    } else {
                        0
                    };

                    writeln!(printer, "populated {}", res)?;
                    writeln!(printer, "frozen {}", 0)?;

                    Ok::<usize, Error>(printer.bytes_written())
                })
                .ok_or(Error::IsDead)?,
            "cgroup.freeze" => self
                .with_inner(|_| {
                    writeln!(printer, "0")?;

                    Ok::<usize, Error>(printer.bytes_written())
                })
                .ok_or(Error::IsDead)?,
            "cgroup.procs" => self
                .with_inner(|processes| {
                    for pid in processes.keys() {
                        writeln!(printer, "{}", pid)?;
                    }

                    Ok::<usize, Error>(printer.bytes_written())
                })
                .ok_or(Error::IsDead)?,
            "cgroup.subtree_control" => {
                let active_set = self.controller.active_set();
                self.with_inner(|_| {
                    writeln!(printer, "{}", active_set)?;

                    Ok::<usize, Error>(printer.bytes_written())
                })
                .ok_or(Error::IsDead)?
            }
            _ => self
                .with_inner(|_| self.controller.read_attr_at(name, offset, writer))
                .ok_or(Error::IsDead)?,
        }
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        match name {
            "cgroup.procs" => {
                let (content, len) = reader
                    .read_cstring_until_end(MAX_ATTR_SIZE)
                    .map_err(|_| Error::PageFault)?;
                let pid = content
                    .to_str()
                    .ok()
                    .and_then(|string| string.trim().parse::<Pid>().ok())
                    .ok_or(Error::InvalidOperation)?;

                with_process_cgroup_locked(pid, |target_process, cgroup_membership| {
                    cgroup_membership.move_process_to_node(target_process, self)
                })?;

                Ok(len)
            }
            "cgroup.subtree_control" => {
                let (activate_set, deactivate_set, len) = read_subtree_control_from_reader(reader)?;

                let mut cgroup_guard = CgroupMembership::write_lock();
                let parent_node = self.cgroup_parent().ok_or(Error::IsDead)?;
                let parent_controller = parent_node.controller();

                self.with_inner(|processes| {
                    if !processes.is_empty() {
                        return Err(Error::ResourceUnavailable);
                    }

                    for ctrl_type in activate_set.iter_types() {
                        self.controller.activate(
                            ctrl_type,
                            self,
                            Some(parent_controller),
                            &mut cgroup_guard,
                        )?;
                    }
                    for ctrl_type in deactivate_set.iter_types() {
                        self.controller
                            .deactivate(ctrl_type, self, &mut cgroup_guard)?;
                    }

                    Ok(len)
                })
                .ok_or(Error::IsDead)?
            }
            _ => self
                .with_inner(|_| self.controller.write_attr(name, reader))
                .ok_or(Error::IsDead)?,
        }
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }

    fn create_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        let _cgroup_read_guard = CgroupMembership::read_lock();
        self.with_inner(|_| {
            let new_child =
                CgroupNode::new(name.to_string().into(), self.depth + 1, &self.controller);
            self.add_child(new_child.clone())?;
            Ok(new_child as _)
        })
        .ok_or(Error::NotFound)?
    }
});

fn with_process_cgroup_locked<F>(pid: Pid, op: F) -> Result<()>
where
    F: FnOnce(Arc<Process>, &mut CgroupMembership) -> Result<()>,
{
    let process = if pid == 0 {
        current!()
    } else {
        pid_table::pid_table_mut()
            .get_process(pid)
            .ok_or(Error::InvalidOperation)?
    };

    let mut cgroup_guard = CgroupMembership::write_lock();
    if process.status().is_zombie() {
        return Err(Error::InvalidOperation);
    }

    op(process, &mut cgroup_guard)
}

fn read_subtree_control_from_reader(
    reader: &mut VmReader,
) -> Result<(SubCtrlSet, SubCtrlSet, usize)> {
    let (content, len) = reader
        .read_cstring_until_end(MAX_ATTR_SIZE)
        .map_err(|_| Error::PageFault)?;
    let content = content.to_str().map_err(|_| Error::InvalidOperation)?;

    let mut activate_set = SubCtrlSet::empty();
    let mut deactivate_set = SubCtrlSet::empty();

    let actions = content.split_whitespace();
    for action in actions {
        if action.len() < 2 {
            return Err(Error::InvalidOperation);
        }

        match action.chars().next() {
            Some('+') => {
                let ctrl_type = SubCtrlType::from_str(&action[1..])?;
                activate_set.add_type(ctrl_type);
                deactivate_set.remove_type(ctrl_type);
            }
            Some('-') => {
                let ctrl_type = SubCtrlType::from_str(&action[1..])?;
                deactivate_set.add_type(ctrl_type);
                activate_set.remove_type(ctrl_type);
            }
            _ => return Err(Error::InvalidOperation),
        };
    }

    Ok((activate_set, deactivate_set, len))
}

pub(crate) trait CgroupSysNode: SysBranchNode {
    fn controller(&self) -> &Controller;

    fn cgroup_parent(&self) -> Option<Arc<dyn CgroupSysNode>> {
        let parent = self.parent()?;
        if parent.is_root() {
            Some(Arc::downcast::<CgroupSystem>(parent).unwrap())
        } else {
            Some(Arc::downcast::<CgroupNode>(parent).unwrap())
        }
    }
}
