use alloc::sync::Arc;

use crate::SystemTime;

pub(crate) trait Driver {
    fn try_new() -> Option<Self>
    where
        Self: Sized;

    fn read_rtc(&self) -> SystemTime;
}

macro_rules! declare_rtc_drivers {
    ( $( #[cfg $cfg:tt ] $module:ident :: $name:ident),* $(,)? ) => {
        pub(super) fn init_rtc_driver() -> Arc<dyn Driver + Send + Sync> {
            $(
                #[cfg $cfg]
                if let Some(driver) = $module::$name::try_new() {
                    return Arc::new(driver);
                }
            )*

            ostd::warn!("No RTC device found, falling back to a dummy RTC");

            Arc::new(RtcDummy)
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod cmos;
#[cfg(target_arch = "riscv64")]
mod goldfish;
#[cfg(target_arch = "x86_64")]
mod kvmclock;
#[cfg(target_arch = "loongarch64")]
mod loongson;
#[cfg(target_arch = "aarch64")]
mod pl031;

declare_rtc_drivers! {
    #[cfg(target_arch = "x86_64")] kvmclock::RtcKvmClock,
    #[cfg(target_arch = "x86_64")] cmos::RtcCmos,
    #[cfg(target_arch = "riscv64")] goldfish::RtcGoldfish,
    #[cfg(target_arch = "loongarch64")] loongson::RtcLoongson,
    #[cfg(target_arch = "aarch64")] pl031::RtcPl031,
}

struct RtcDummy;

impl Driver for RtcDummy {
    fn try_new() -> Option<Self> {
        Some(Self)
    }

    fn read_rtc(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}
