use alloc::{collections::vec_deque::VecDeque, sync::Arc};
use core::{
    fmt::Display,
    str::FromStr,
    sync::atomic::{AtomicU8, Ordering},
};

use aster_systree::{Error, Result, SysAttrSetBuilder, SysBranchNode, SysObj};
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Rcu,
};

use crate::fs::cgroupfs::{
    CgroupMembership, CgroupNode,
    controller::{
        cpu::CpuController, cpuset::CpuSetController, memory::MemoryController,
        pids::PidsController,
    },
    systree_node::CgroupSysNode,
};

pub(super) mod cpu;
mod cpuset;
mod memory;
mod pids;

trait SubControl {
    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr_at(&self, name: &str, offset: usize, writer: &mut VmWriter) -> Result<usize>;

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize>;
}

trait SubControlStatic: SubControl + Sized + 'static {
    fn new(is_root: bool, is_active: bool) -> Self;

    fn type_() -> SubCtrlType;

    fn read_from(controller: &Controller) -> Arc<SubController<Self>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SubCtrlType {
    CpuSet,
    Cpu,
    Memory,
    Pids,
}

impl SubCtrlType {
    const ALL: [Self; 4] = [Self::CpuSet, Self::Cpu, Self::Memory, Self::Pids];

    const fn as_str(self) -> &'static str {
        match self {
            Self::CpuSet => "cpuset",
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Pids => "pids",
        }
    }
}

impl FromStr for SubCtrlType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "cpuset" => Ok(SubCtrlType::CpuSet),
            "cpu" => Ok(SubCtrlType::Cpu),
            "memory" => Ok(SubCtrlType::Memory),
            "pids" => Ok(SubCtrlType::Pids),
            _ => Err(Error::NotFound),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(super) struct SubCtrlSet: u8 {
        const CPUSET = 1 << 0;
        const CPU    = 1 << 1;
        const MEMORY = 1 << 2;
        const PIDS   = 1 << 3;
    }
}

impl SubCtrlSet {
    pub(super) fn contains_type(&self, ctrl_type: SubCtrlType) -> bool {
        self.contains(ctrl_type.into())
    }

    pub(super) fn add_type(&mut self, ctrl_type: SubCtrlType) {
        *self |= ctrl_type.into()
    }

    pub(super) fn remove_type(&mut self, ctrl_type: SubCtrlType) {
        *self -= ctrl_type.into()
    }

    pub(super) fn iter_types(&self) -> impl Iterator<Item = SubCtrlType> + '_ {
        SubCtrlType::ALL
            .into_iter()
            .filter(|&ctrl_type| self.contains_type(ctrl_type))
    }
}

impl Display for SubCtrlSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut iter = self.iter_types();
        if let Some(first) = iter.next() {
            write!(f, "{}", first.as_str())?;
            for ctrl_type in iter {
                write!(f, " {}", ctrl_type.as_str())?;
            }
        }

        Ok(())
    }
}

impl From<SubCtrlType> for SubCtrlSet {
    fn from(ctrl_type: SubCtrlType) -> Self {
        match ctrl_type {
            SubCtrlType::CpuSet => Self::CPUSET,
            SubCtrlType::Cpu => Self::CPU,
            SubCtrlType::Memory => Self::MEMORY,
            SubCtrlType::Pids => Self::PIDS,
        }
    }
}

impl From<u8> for SubCtrlSet {
    fn from(value: u8) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<SubCtrlSet> for u8 {
    fn from(value: SubCtrlSet) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(SubCtrlSet, {
    #[derive(Debug)]
    struct AtomicSubCtrlSet(AtomicU8);
});

struct SubController<T: SubControlStatic> {
    inner: Option<T>,
    parent: Option<Arc<SubController<T>>>,
}

impl<T: SubControlStatic> SubController<T> {
    fn new(parent_controller: Option<&Controller>) -> Self {
        let is_root = parent_controller.is_none();
        let is_active = if let Some(parent) = parent_controller {
            parent.active_set().contains_type(T::type_())
        } else {
            true
        };

        let inner = if is_active || T::type_() == SubCtrlType::Cpu {
            Some(T::new(is_root, is_active))
        } else {
            None
        };

        let parent = parent_controller.map(T::read_from);

        Self { inner, parent }
    }
}

trait TryGetSubControl {
    fn try_get(&self) -> Option<&dyn SubControl>;
}

impl<T: SubControlStatic> TryGetSubControl for SubController<T> {
    fn try_get(&self) -> Option<&dyn SubControl> {
        self.inner.as_ref().map(|sub_ctrl| sub_ctrl as _)
    }
}

pub(crate) struct Controller {
    active_set: AtomicSubCtrlSet,

    cpuset: Rcu<Arc<SubController<CpuSetController>>>,
    cpu: Rcu<Arc<SubController<CpuController>>>,
    memory: Rcu<Arc<SubController<MemoryController>>>,
    pids: Rcu<Arc<SubController<PidsController>>>,
}

impl Controller {
    pub(super) fn new(parent_controller: Option<&Controller>) -> Self {
        let cpuset_controller = Arc::new(SubController::new(parent_controller));
        let cpu_controller = Arc::new(SubController::new(parent_controller));
        let memory_controller = Arc::new(SubController::new(parent_controller));
        let pids_controller = Arc::new(SubController::new(parent_controller));

        Self {
            active_set: AtomicSubCtrlSet::new(SubCtrlSet::empty()),
            cpuset: Rcu::new(cpuset_controller),
            cpu: Rcu::new(cpu_controller),
            memory: Rcu::new(memory_controller),
            pids: Rcu::new(pids_controller),
        }
    }

    pub(super) fn init_attr_set(builder: &mut SysAttrSetBuilder, is_root: bool) {
        CpuSetController::init_attr_set(builder, is_root);
        CpuController::init_attr_set(builder, is_root);
        MemoryController::init_attr_set(builder, is_root);
        PidsController::init_attr_set(builder, is_root);
    }

    fn read_sub(&self, ctrl_type: SubCtrlType) -> Arc<dyn TryGetSubControl> {
        match ctrl_type {
            SubCtrlType::CpuSet => CpuSetController::read_from(self),
            SubCtrlType::Cpu => CpuController::read_from(self),
            SubCtrlType::Memory => MemoryController::read_from(self),
            SubCtrlType::Pids => PidsController::read_from(self),
        }
    }

    pub(super) fn is_attr_absent(&self, name: &str) -> bool {
        let Some((subsys, _)) = name.split_once('.') else {
            return false;
        };
        let Ok(ctrl_type) = SubCtrlType::from_str(subsys) else {
            return false;
        };

        let sub_controller = self.read_sub(ctrl_type);
        let Some(controller) = sub_controller.try_get() else {
            return true;
        };

        controller.is_attr_absent(name)
    }

    pub(super) fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };
        let ctrl_type = SubCtrlType::from_str(subsys)?;

        let sub_controller = self.read_sub(ctrl_type);
        let Some(controller) = sub_controller.try_get() else {
            return Err(Error::IsDead);
        };

        controller.read_attr_at(name, offset, writer)
    }

    pub(super) fn write_attr(&self, name: &str, reader: &mut VmReader) -> Result<usize> {
        let Some((subsys, _)) = name.split_once('.') else {
            return Err(Error::NotFound);
        };
        let ctrl_type = SubCtrlType::from_str(subsys)?;

        let sub_controller = self.read_sub(ctrl_type);
        let Some(controller) = sub_controller.try_get() else {
            return Err(Error::IsDead);
        };

        controller.write_attr(name, reader)
    }

    pub(super) fn activate(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        parent_controller: Option<&Controller>,
        cgroup_membership: &mut CgroupMembership,
    ) -> Result<()> {
        let mut active_set = self.active_set();
        if active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        if parent_controller
            .is_some_and(|controller| !controller.active_set().contains_type(ctrl_type))
        {
            return Err(Error::NotFound);
        }

        active_set.add_type(ctrl_type);
        self.active_set.store(active_set, Ordering::Relaxed);
        self.update_sub_controllers_for_descents(ctrl_type, current_node, cgroup_membership);

        Ok(())
    }

    pub(super) fn deactivate(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        cgroup_membership: &mut CgroupMembership,
    ) -> Result<()> {
        let mut active_set = self.active_set();
        if !active_set.contains_type(ctrl_type) {
            return Ok(());
        }

        for child in current_node.children() {
            let cgroup_child = Arc::downcast::<CgroupNode>(child).unwrap();
            if cgroup_child
                .controller()
                .active_set()
                .contains_type(ctrl_type)
            {
                return Err(Error::InvalidOperation);
            }
        }

        active_set.remove_type(ctrl_type);
        self.active_set.store(active_set, Ordering::Relaxed);
        self.update_sub_controllers_for_descents(ctrl_type, current_node, cgroup_membership);

        Ok(())
    }

    fn update_sub_controllers_for_descents(
        &self,
        ctrl_type: SubCtrlType,
        current_node: &dyn CgroupSysNode,
        cgroup_membership: &mut CgroupMembership,
    ) {
        fn update_sub_controller_for_one_child(
            child: &Arc<dyn SysObj>,
            ctrl_type: SubCtrlType,
            parent_controller: &Controller,
            cgroup_membership: &mut CgroupMembership,
        ) {
            let child_node = child.as_any().downcast_ref::<CgroupNode>().unwrap();
            match ctrl_type {
                SubCtrlType::CpuSet => {
                    let new_controller = Arc::new(SubController::new(Some(parent_controller)));
                    child_node.controller().cpuset.update(new_controller);
                }
                SubCtrlType::Cpu => {
                    let mut new_controller: SubController<CpuController> =
                        SubController::new(Some(parent_controller));
                    {
                        let guard = child_node.controller().cpu.read();
                        let previous_controller = guard.get();
                        new_controller
                            .inner
                            .as_mut()
                            .unwrap()
                            .init_stats(previous_controller.inner.as_ref().unwrap());
                    }
                    child_node.controller().cpu.update(Arc::new(new_controller));
                }
                SubCtrlType::Memory => {
                    let new_controller = Arc::new(SubController::new(Some(parent_controller)));
                    child_node.controller().memory.update(new_controller);
                }
                SubCtrlType::Pids => {
                    let mut new_controller: SubController<PidsController> =
                        SubController::new(Some(parent_controller));
                    if let Some(inner) = new_controller.inner.as_mut() {
                        let count = cgroup_membership.count_subtree_processes(child_node);
                        if count > 0 {
                            inner.init_count(count);
                        }
                    }
                    child_node
                        .controller()
                        .pids
                        .update(Arc::new(new_controller));
                }
            }
        }

        let mut descents = VecDeque::new();

        current_node.visit_children_with(0, &mut |child_node| {
            descents.push_back(child_node.clone());
            update_sub_controller_for_one_child(child_node, ctrl_type, self, cgroup_membership);

            Some(())
        });

        while let Some(node) = descents.pop_front() {
            let current_node = Arc::downcast::<CgroupNode>(node).unwrap();
            let current_controller = current_node.controller();
            current_node.visit_children_with(0, &mut |child_node| {
                descents.push_back(child_node.clone());
                update_sub_controller_for_one_child(
                    child_node,
                    ctrl_type,
                    current_controller,
                    cgroup_membership,
                );

                Some(())
            });
        }
    }

    pub(super) fn active_set(&self) -> SubCtrlSet {
        self.active_set.load(Ordering::Relaxed)
    }
}

impl Controller {
    pub(super) fn charge_pids(&self) {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.charge_hierarchy();
    }

    pub(crate) fn pre_charge_pids<'a>(
        &'a self,
        _cgroup_membership: &'a CgroupMembership,
    ) -> Result<PidsPreCharge<'a>, TryChargeError> {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.try_charge_hierarchy()?;

        Ok(PidsPreCharge { controller: self })
    }

    pub(super) fn uncharge_pids(&self) {
        let guard = self.pids.read();
        let sub = guard.get();
        sub.uncharge_hierarchy();
    }
}

pub(crate) struct PidsPreCharge<'a> {
    controller: &'a Controller,
}

impl PidsPreCharge<'_> {
    pub(super) fn apply(self) {
        core::mem::forget(self);
    }
}

impl Drop for PidsPreCharge<'_> {
    fn drop(&mut self) {
        self.controller.uncharge_pids();
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TryChargeError;
