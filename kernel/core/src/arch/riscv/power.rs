crate::register_restart_handler!(
    ostd::arch::power::try_restart,
    crate::power::Priority::FIRMWARE
);
crate::register_poweroff_handler!(
    ostd::arch::power::try_poweroff,
    crate::power::Priority::FIRMWARE
);
