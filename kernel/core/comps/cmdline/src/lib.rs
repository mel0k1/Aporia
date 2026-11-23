#![no_std]
#![deny(unsafe_code)]
#![feature(int_from_ascii)]
#![feature(const_convert)]
#![feature(const_trait_impl)]

extern crate alloc;

macro_rules! __log_prefix {
    () => {
        "cmdline: "
    };
}

mod dispatch;
mod early;
pub mod parse;
pub mod types;
mod unimplemented;

pub use dispatch::INIT_PROC_ARGS;
#[doc(hidden)]
pub use dispatch::KernelParam;
#[doc(hidden)]
pub use inventory::submit;
#[doc(hidden)]
pub use unimplemented::setup_unimplemented;

#[macro_export]
macro_rules! define_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_kv_param);
    };
}

#[macro_export]
macro_rules! define_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_kv_param);
    };
}

#[macro_export]
macro_rules! define_repeatable_kv_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_repeatable_kv_param);
    };
}

#[macro_export]
macro_rules! define_repeatable_kv_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_repeatable_kv_param);
    };
}

#[macro_export]
macro_rules! define_flag_param {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@late, $name, $storage, $crate::parse::setup_flag_param);
    };
}

#[macro_export]
macro_rules! define_flag_param_early {
    ($name:expr, $storage:expr) => {
        $crate::__define_param!(@early, $name, $storage, $crate::parse::setup_flag_param);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __define_param {
    (@late, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[Option<&str>]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam::new($name, __kparam_setup, false)
            }
        };
    };

    (@early, $name:expr, $storage:expr, $setup:path) => {
        const _: () = {
            fn __kparam_setup(occurrences: &[Option<&str>]) {
                $setup(&$storage, occurrences, $name);
            }
            $crate::submit! {
                $crate::KernelParam::new($name, __kparam_setup, true)
            }
        };
    };
}
