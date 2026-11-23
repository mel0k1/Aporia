pub mod hooks;
mod modules;

pub(crate) mod yama {
    pub(crate) use super::modules::yama::{YamaScope, get_scope, set_scope};
}

use self::hooks::{LsmAlienAccessHook, LsmCapabilityHook};
use crate::prelude::*;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct LsmFlags: u32 {
        const LEGACY_MAJOR = 1 << 0;
        const EXCLUSIVE    = 1 << 1;
    }
}

trait LsmModule: LsmAlienAccessHook + LsmCapabilityHook + Sync {
    fn name(&self) -> &'static str;

    fn flags(&self) -> LsmFlags;
}

pub(crate) fn is_yama_enabled() -> bool {
    modules::active_modules()
        .iter()
        .any(|module| module.name() == "yama")
}

pub(super) fn init() {
    for module in modules::active_modules() {
        info!("[kernel] LSM module enabled: {}", module.name());
    }
}
