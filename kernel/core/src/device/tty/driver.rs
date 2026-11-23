use crate::{
    device::tty::{Tty, termio::CTermios},
    fs::{devtmpfs::DevtmpfsNodeMeta, file::PerOpenFileOps},
    prelude::*,
    util::ioctl::RawIoctl,
};

pub(crate) trait TtyDriver: Send + Sync + 'static {
    const DEVICE_MAJOR_ID: u32;

    fn devtmpfs_meta(&self, _index: u32) -> Option<DevtmpfsNodeMeta>;

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn PerOpenFileOps>>
    where
        Self: Sized;

    fn push_output(&self, chs: &[u8]) -> Result<usize>;

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_;

    fn can_push(&self) -> bool;

    fn notify_input(&self);

    fn on_termios_change(&self, old_termios: &CTermios, new_termios: &CTermios);

    fn ioctl(&self, _tty: &Tty<Self>, _raw_ioctl: RawIoctl) -> Result<bool>
    where
        Self: Sized,
    {
        Ok(false)
    }
}
