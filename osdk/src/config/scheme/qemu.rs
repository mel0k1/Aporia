use std::{path::PathBuf, process};

use crate::{
    arch::{Arch, get_default_arch},
    config::unix_args::{apply_kv_array, get_key, split_to_kv_array},
    error::Errno,
    error_msg,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct QemuScheme {
    pub args: Option<String>,
    pub bootdev_append_options: Option<String>,
    pub path: Option<PathBuf>,
    pub with_monitor: Option<bool>,
    pub log_file: Option<PathBuf>,
    pub with_daemons: Option<Vec<DaemonScheme>>,
}

#[derive(Clone, Debug, Deserialize, Eq, Serialize)]
pub struct Qemu {
    pub args: String,
    pub bootdev_append_options: Option<String>,
    pub path: PathBuf,
    pub with_monitor: bool,
    pub log_file: Option<PathBuf>,
    #[serde(default)]
    pub with_daemons: Vec<DaemonScheme>,
}

impl Default for Qemu {
    fn default() -> Self {
        Qemu {
            args: String::new(),
            bootdev_append_options: None,
            path: PathBuf::from(get_default_arch().system_qemu()),
            with_monitor: false,
            log_file: None,
            with_daemons: Vec::new(),
        }
    }
}

impl PartialEq for Qemu {
    fn eq(&self, other: &Self) -> bool {
        fn strip_numbers(input: &str) -> String {
            input.chars().filter(|c| !c.is_numeric()).collect()
        }

        strip_numbers(&self.args) == strip_numbers(&other.args)
            && self.bootdev_append_options == other.bootdev_append_options
            && self.path == other.path
            && self.with_monitor == other.with_monitor
            && self.log_file == other.log_file
            && self.with_daemons == other.with_daemons
    }
}

impl Qemu {
    pub fn apply_qemu_args(&mut self, args: &Vec<String>) {
        let mut joined = split_to_kv_array(&self.args);

        for arg in joined.iter() {
            check_qemu_arg(arg);
        }

        apply_kv_array(&mut joined, args, " ", MULTI_VALUE_KEYS);

        self.args = joined.join(" ");
    }
}

impl QemuScheme {
    pub fn inherit(&mut self, from: &Self) {
        if self.args.is_none() {
            self.args.clone_from(&from.args);
        }
        if self.path.is_none() {
            self.path.clone_from(&from.path);
        }
        if self.with_monitor.is_none() {
            self.with_monitor.clone_from(&from.with_monitor);
        }
        if self.log_file.is_none() {
            self.log_file.clone_from(&from.log_file);
        }
        if self.with_daemons.is_none() {
            self.with_daemons.clone_from(&from.with_daemons);
        }
    }

    pub fn finalize(self, arch: Arch) -> Qemu {
        Qemu {
            args: self.args.unwrap_or_default(),
            bootdev_append_options: self.bootdev_append_options,
            path: self.path.unwrap_or(PathBuf::from(arch.system_qemu())),
            with_monitor: self.with_monitor.unwrap_or(false),
            log_file: self.log_file,
            with_daemons: self.with_daemons.unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DaemonScheme {
    pub(crate) path: PathBuf,
    #[serde(default)]
    pub(crate) args: Vec<String>,
}

const MULTI_VALUE_KEYS: &[&str] = &[
    "-device", "-chardev", "-object", "-netdev", "-drive", "-cdrom",
];
const SINGLE_VALUE_KEYS: &[&str] = &["-cpu", "-machine", "-m", "-serial", "-monitor", "-display"];
const NO_VALUE_KEYS: &[&str] = &["--no-reboot", "-nographic", "-enable-kvm"];
const NOT_ALLOWED_TO_SET_KEYS: &[&str] = &["-kernel", "-append", "-initrd"];

fn check_qemu_arg(arg: &str) {
    let key = if let Some(key) = get_key(arg, " ") {
        key
    } else {
        arg.to_string()
    };

    if NOT_ALLOWED_TO_SET_KEYS.contains(&key.as_str()) {
        error_msg!("`{}` is not allowed to set", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if NO_VALUE_KEYS.contains(&key.as_str()) && key.as_str() != arg {
        error_msg!("`{}` cannot have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }

    if (SINGLE_VALUE_KEYS.contains(&key.as_str()) || MULTI_VALUE_KEYS.contains(&key.as_str()))
        && key.as_str() == arg
    {
        error_msg!("`{}` should have value", arg);
        process::exit(Errno::ParseMetadata as _);
    }
}
