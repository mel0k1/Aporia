use std::path::PathBuf;

use clap::ValueEnum;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootScheme {
    #[serde(default)]
    pub kcmd_args: Vec<String>,
    #[serde(default)]
    pub init_args: Vec<String>,
    pub initramfs: Option<PathBuf>,
    pub method: Option<BootMethod>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum BootMethod {
    GrubRescueIso,
    GrubQcow2,
    #[default]
    VmmDirect,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Boot {
    pub kcmdline: Vec<String>,
    pub initramfs: Option<PathBuf>,
    pub method: BootMethod,
}

impl BootScheme {
    pub fn inherit(&mut self, from: &Self) {
        self.kcmd_args = {
            let mut kcmd_args = from.kcmd_args.clone();
            kcmd_args.extend(self.kcmd_args.clone());
            kcmd_args
        };
        self.init_args = {
            let mut init_args = from.init_args.clone();
            init_args.extend(self.init_args.clone());
            init_args
        };
        if self.initramfs.is_none() {
            self.initramfs.clone_from(&from.initramfs);
        }
        if self.method.is_none() {
            self.method = from.method;
        }
    }

    pub fn finalize(self) -> Boot {
        let mut kcmdline = self.kcmd_args;
        kcmdline.push("--".to_owned());
        kcmdline.extend(self.init_args);
        Boot {
            kcmdline,
            initramfs: self.initramfs,
            method: self.method.unwrap_or(BootMethod::VmmDirect),
        }
    }
}
