#[macro_export]
macro_rules! define_unimplemented_param {
    ($($name:expr),+ $(,)?) => {
        $(
            const _: () = {
                fn __kparam_setup(occurrences: &[Option<&str>]) {
                    $crate::setup_unimplemented(occurrences, $name);
                }
                $crate::submit! {
                    $crate::KernelParam::new($name, __kparam_setup, false)
                }
            };
        )+
    };
}

#[doc(hidden)]
pub fn setup_unimplemented(occurrences: &[Option<&str>], name: &str) {
    if !occurrences.is_empty() {
        ostd::warn!("kernel parameter '{}' is not yet implemented", name);
    }
}

define_unimplemented_param!(
    "tsc",
    "no_timer_check",
    "reboot",
    "pci",
    "debug",
    "panic",
    "nr_cpus",
    "selinux",
    "initrd",
    "noreplace_smp",
    "initcall_debug"
);
