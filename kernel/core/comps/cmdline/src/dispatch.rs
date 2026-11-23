use alloc::{
    collections::BTreeMap,
    ffi::CString,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use component::{ComponentInitError, init_component};
use spin::Once;

use crate::early::is_early_param;

#[derive(Debug, PartialEq)]
pub struct InitprocArgs {
    argv: Vec<CString>,
    envp: Vec<CString>,
}

impl InitprocArgs {
    pub fn argv(&self) -> &[CString] {
        &self.argv
    }

    pub fn envp(&self) -> &[CString] {
        &self.envp
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct KernelParam {
    name: &'static str,
    setup_fn: fn(occurrences: &[Option<&str>]),
    early: bool,
}

impl KernelParam {
    #[doc(hidden)]
    pub const fn new(
        name: &'static str,
        setup_fn: fn(occurrences: &[Option<&str>]),
        early: bool,
    ) -> KernelParam {
        if Self::contains_hyphen(name) {
            panic!("kernel param registration must not contain '-' (use '_')");
        }
        KernelParam {
            name,
            setup_fn,
            early,
        }
    }

    const fn contains_hyphen(s: &'static str) -> bool {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'-' {
                return true;
            }
            i += 1;
        }
        false
    }
}

inventory::collect!(KernelParam);

pub static INIT_PROC_ARGS: Once<InitprocArgs> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    INIT_PROC_ARGS.call_once(|| dispatch_params(ostd::boot::boot_info().kernel_cmdline.as_str()));

    Ok(())
}

fn split_arg(input: &str) -> impl Iterator<Item = &str> {
    let mut inside_quotes = false;

    input.split(move |c: char| {
        if c == '"' {
            inside_quotes = !inside_quotes;
        }

        !inside_quotes && c.is_whitespace()
    })
}

fn dispatch_params(cmdline: &str) -> InitprocArgs {
    let mut result: InitprocArgs = InitprocArgs {
        argv: Vec::new(),
        envp: Vec::new(),
    };

    let mut kcmdline_end = false;

    let mut registry = BTreeMap::new();
    for p in inventory::iter::<KernelParam> {
        if let Some(prev) = registry.insert(p.name, p) {
            ostd::warn!(
                "duplicate kernel parameter '{}' registered; keeping last",
                prev.name
            );
        }
    }

    let mut grouped: BTreeMap<String, (Vec<Option<&str>>, usize)> = BTreeMap::new();
    for (pos, arg) in split_arg(cmdline).enumerate() {
        if kcmdline_end {
            result.argv.push(CString::new(arg).unwrap());
            continue;
        }
        if arg == "--" {
            kcmdline_end = true;
            continue;
        }

        let (key, value) = match arg.find('=') {
            Some(pos) => (&arg[..pos], Some(&arg[pos + 1..])),
            None => (arg, None),
        };
        let normalized = key.replace('-', "_");

        if registry.contains_key(normalized.as_str()) {
            grouped
                .entry(normalized)
                .and_modify(|occurrences| {
                    occurrences.0.push(value);
                    occurrences.1 = pos;
                })
                .or_insert_with(|| (vec![value], pos));
        } else if is_early_param(normalized.as_str()) {
            continue;
        } else {
            if key.contains('.') {
                continue;
            } else if let Some(value) = value {
                let envp_entry = CString::new(key.to_string() + "=" + value).unwrap();
                result.envp.push(envp_entry);
            } else {
                let argv_entry = CString::new(key.to_string()).unwrap();
                result.argv.push(argv_entry);
            }
        }
    }

    let (mut early_params, mut params): (Vec<_>, Vec<_>) = grouped
        .iter()
        .filter_map(|(name, occurrences)| {
            registry
                .get(name.as_str())
                .map(|param| (*param, occurrences.1, occurrences.0.as_slice()))
        })
        .partition(|(param, _, _)| param.early);

    for pending_params in [&mut early_params, &mut params] {
        pending_params.sort_by_key(|(_, last_pos, _)| *last_pos);
    }

    early_params
        .into_iter()
        .chain(params)
        .for_each(|(param, _, occurrences)| (param.setup_fn)(occurrences));

    result
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn unknown_kv_forwarded_to_init_env() {
        let args = dispatch_params("unknown_key=1");
        assert!(args.envp().iter().any(|e| e.to_bytes() == b"unknown_key=1"));
    }

    #[ktest]
    fn dotted_unknown_param_not_forwarded() {
        let args = dispatch_params("some_module.flag");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
    }

    #[ktest]
    fn repeated_unknown_kv_and_arg_forwarding() {
        let args = dispatch_params("unknown_key=1 unknown_key unknown_key=2");

        assert_eq!(args.envp.len(), 2);
        assert_eq!(args.envp[0].to_bytes(), b"unknown_key=1");
        assert_eq!(args.envp[1].to_bytes(), b"unknown_key=2");

        assert_eq!(args.argv.len(), 1);
        assert_eq!(args.argv[0].to_bytes(), b"unknown_key");
    }

    #[ktest]
    fn early_params_not_forwarded() {
        let args = dispatch_params("earlycon loglevel=3");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
    }

    #[ktest]
    fn registered_params_not_forwarded() {
        static TEST_VARIABLE: Once<u32> = Once::new();
        static TEST_VARIABLE_REPEATED: Once<Vec<String>> = Once::new();

        crate::define_kv_param!("log_level", TEST_VARIABLE);
        crate::define_repeatable_kv_param!("console", TEST_VARIABLE_REPEATED);

        let args = dispatch_params("log_level=4 console=ttyS0 console=ttyS1");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
    }

    #[ktest]
    fn registered_params_dispatched_by_last_position() {
        use core::sync::atomic::{AtomicU8, Ordering};

        struct SetOne;
        struct SetTwo;

        static VALUE: AtomicU8 = AtomicU8::new(0);
        static SET_ONE: SetOne = SetOne;
        static SET_TWO: SetTwo = SetTwo;

        impl crate::parse::ParamStorage for SetOne {
            type Value = bool;

            fn store_param(&self, value: Self::Value) {
                if value {
                    VALUE.store(1, Ordering::Relaxed);
                }
            }
        }

        impl crate::parse::ParamStorage for SetTwo {
            type Value = bool;

            fn store_param(&self, value: Self::Value) {
                if value {
                    VALUE.store(2, Ordering::Relaxed);
                }
            }
        }

        crate::define_flag_param!("test_set_one", SET_ONE);
        crate::define_flag_param!("test_set_two", SET_TWO);

        let args = dispatch_params("test_set_one test_set_two");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
        assert_eq!(VALUE.load(Ordering::Relaxed), 2);

        let args = dispatch_params("test_set_two test_set_one");
        assert!(args.argv().is_empty());
        assert!(args.envp().is_empty());
        assert_eq!(VALUE.load(Ordering::Relaxed), 1);
    }
}
