use std::path::PathBuf;

use crate::arch::Arch;

mod action;
pub use action::*;
mod boot;
pub use boot::*;
mod grub;
pub use grub::*;
mod qemu;
pub use qemu::*;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Scheme {
    pub work_dir: Option<PathBuf>,
    #[serde(default)]
    pub supported_archs: Vec<Arch>,
    pub boot: Option<BootScheme>,
    pub grub: Option<GrubScheme>,
    pub qemu: Option<QemuScheme>,
    pub build: Option<BuildScheme>,
    pub run: Option<ActionScheme>,
    pub test: Option<ActionScheme>,
}

macro_rules! inherit_optional {
    ($from:ident, $to:ident, .$field:ident) => {
        if let Some(ref mut to_field) = $to.$field {
            if let Some(from_field) = &$from.$field {
                to_field.inherit(from_field);
            }
        } else {
            $to.$field = $from.$field.clone();
        }
    };
}
use inherit_optional;

impl Scheme {
    pub fn empty() -> Self {
        Scheme {
            work_dir: None,
            supported_archs: vec![],
            boot: None,
            grub: None,
            qemu: None,
            build: None,
            run: None,
            test: None,
        }
    }

    pub fn inherit(&mut self, from: &Self) {
        inherit_optional!(from, self, .boot);
        inherit_optional!(from, self, .grub);
        inherit_optional!(from, self, .build);
        inherit_optional!(from, self, .run);
        inherit_optional!(from, self, .test);
        if let Some(qemu) = &mut self.qemu {
            if let Some(from_qemu) = &from.qemu {
                if qemu.args.is_none() {
                    qemu.args.clone_from(&from_qemu.args);
                    self.work_dir.clone_from(&from.work_dir);
                }
                if qemu.path.is_none() {
                    qemu.path.clone_from(&from_qemu.path);
                    self.work_dir.clone_from(&from.work_dir);
                }
                if qemu.log_file.is_none() {
                    qemu.log_file.clone_from(&from_qemu.log_file);
                }
                if qemu.with_daemons.is_none() {
                    qemu.with_daemons.clone_from(&from_qemu.with_daemons);
                }
            }
        } else {
            self.qemu.clone_from(&from.qemu);
            self.work_dir.clone_from(&from.work_dir);
        }
    }

    pub fn run_and_test_inherit_from_global(&mut self) {
        let old_scheme = self.clone();

        if let Some(run) = &mut self.run {
            inherit_optional!(old_scheme, run, .boot);
            inherit_optional!(old_scheme, run, .grub);
            inherit_optional!(old_scheme, run, .qemu);
            inherit_optional!(old_scheme, run, .build);
        } else {
            self.run = Some(ActionScheme {
                boot: self.boot.clone(),
                grub: self.grub.clone(),
                qemu: self.qemu.clone(),
                build: self.build.clone(),
            });
        }

        if let Some(test) = &mut self.test {
            inherit_optional!(old_scheme, test, .boot);
            inherit_optional!(old_scheme, test, .grub);
            inherit_optional!(old_scheme, test, .qemu);
            inherit_optional!(old_scheme, test, .build);
        } else {
            self.test = Some(ActionScheme {
                boot: self.boot.clone(),
                grub: self.grub.clone(),
                qemu: self.qemu.clone(),
                build: self.build.clone(),
            });
        }
    }
}
