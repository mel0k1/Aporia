mod evdev;
mod fb;
mod mem;
pub(crate) mod misc;
mod model;
mod pty;
pub mod registry;
pub(crate) mod tty;

use device_id::DeviceId;
pub(crate) use mem::{getrandom, geturandom};
pub(crate) use pty::{PtyMaster, PtySlave, new_pty_pair};
pub(crate) use registry::lookup;

use crate::{
    fs::{devtmpfs::DevtmpfsNodeMeta, file::PerOpenFileOps},
    prelude::*,
};

pub trait Device: Send + Sync + 'static {
    fn type_(&self) -> DeviceType;

    fn id(&self) -> DeviceId;

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta>;

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>>;
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.type_())
            .field("id", &self.id())
            .field("devtmpfs_meta", &self.devtmpfs_meta())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum DeviceType {
    Char,
    Block,
}

pub(crate) fn init_in_first_kthread() {
    model::init_in_first_kthread();
    registry::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    fb::init_in_first_kthread();
}

pub(crate) fn init_in_first_process() -> Result<()> {
    tty::init_in_first_process()?;
    registry::init_in_first_process()?;

    Ok(())
}
