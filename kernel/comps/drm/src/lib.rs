#![no_std]
#![deny(unsafe_code)]

use alloc::sync::Arc;

use aster_core::{
    device::{Device, registry::char},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks::{self as lsm_hook, CapableContext},
};
use ostd::task::Task;

use crate::{
    device::{DrmDevice, DrmFeatures, RegisteredDrmDevice},
    minor::{DrmMinor, DrmMinorType},
};

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

macro_rules! __log_prefix {
    () => {
        "drm: "
    };
}

pub mod device;
mod file;
pub mod gem;
mod ioctl;
pub mod kms;
mod minor;
pub mod utils;

pub fn register_device(device: Arc<dyn DrmDevice>) -> Result<()> {
    let registered_device = Arc::new(RegisteredDrmDevice::new(device)?);
    let render_minor = if registered_device.device().has_features(DrmFeatures::RENDER) {
        let minor = DrmMinor::new(registered_device.clone(), DrmMinorType::Render);
        char::register(minor.clone())?;
        Some(minor)
    } else {
        None
    };

    let primary_minor = DrmMinor::new(registered_device, DrmMinorType::Primary);

    if let Err(error) = char::register(primary_minor) {
        if let Some(render_minor) = render_minor {
            let _ = char::unregister(render_minor.id());
        }
        return Err(error);
    }

    Ok(())
}

fn has_current_sys_admin() -> bool {
    let task = Task::current().unwrap();
    let posix_thread = task.as_posix_thread().unwrap();

    lsm_hook::on_capable(CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .is_ok()
}
