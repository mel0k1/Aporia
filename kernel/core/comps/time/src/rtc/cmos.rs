//! CMOS RTC.
//!
//! "CMOS" is a tiny bit of very low power static memory that lives on the same chip as the
//! Real-Time Clock (RTC).
//!
//! According to the Linux implementation, in x86, if the CMOS/RTC is present at the legacy
//! addresses (I/O Ports 0x70 and 0x71), then it is an MC146818 CMOS/RTC. Therefore, in this
//! module, the register addresses and data interpretation are based on the MC146818 datasheet.
//!
//! Reference:
//! <https://elixir.bootlin.com/linux/v6.17.5/source/arch/x86/kernel/rtc.c#L69>
//! <https://www.scs.stanford.edu/23wi-cs212/pintos/specs/mc146818a.pdf>

use core::num::NonZeroU8;

use ostd::{
    arch::{
        device::io_port::{ReadWriteAccess, WriteOnlyAccess},
        kernel::ACPI_INFO,
    },
    io::IoPort,
    sync::SpinLock,
    warn,
};

use super::Driver;
use crate::SystemTime;

pub(super) struct RtcCmos {
    access: SpinLock<CmosAccess>,
    status_b: StatusB,
}

impl Driver for RtcCmos {
    fn try_new() -> Option<Self> {
        const IOPORT_SEL: u16 = 0x70;
        const IOPORT_VAL: u16 = 0x71;

        let acpi_info = ACPI_INFO.get().unwrap();

        if acpi_info
            .boot_flags
            .is_some_and(|flags| flags.use_time_and_alarm_namespace_for_rtc())
        {
            return None;
        }

        let (io_sel, io_val) = match (IoPort::acquire(IOPORT_SEL), IoPort::acquire(IOPORT_VAL)) {
            (Ok(io_sel), Ok(io_val)) => (io_sel, io_val),
            _ => {
                warn!("Failed to acquire CMOS RTC PIO region");
                return None;
            }
        };

        let century_register = acpi_info.century_register;

        let mut access = CmosAccess {
            io_sel,
            io_val,
            century_register,
        };
        let status_b = access.read_status_b();

        if !access.check_presence() {
            warn!("CMOS RTC reports unexpected status, ignoring this device");
            return None;
        }

        Some(Self {
            access: SpinLock::new(access),
            status_b,
        })
    }

    fn read_rtc(&self) -> SystemTime {
        let cmos_data = CmosData::read_rtc(self);

        let Some(date_time) = chrono::NaiveDate::from_ymd_opt(
            i32::from(cmos_data.year),
            u32::from(cmos_data.month),
            u32::from(cmos_data.day),
        )
        .and_then(|date| {
            date.and_hms_opt(
                u32::from(cmos_data.hour),
                u32::from(cmos_data.minute),
                u32::from(cmos_data.second),
            )
        }) else {
            warn!("CMOS RTC reports an invalid date time, falling back to the Unix epoch");
            return SystemTime::UNIX_EPOCH;
        };

        let duration = date_time
            .signed_duration_since(chrono::DateTime::UNIX_EPOCH.naive_utc())
            .to_std()
            .unwrap_or_default();
        SystemTime::new(duration)
    }
}

struct CmosAccess {
    io_sel: IoPort<u8, WriteOnlyAccess>,
    io_val: IoPort<u8, ReadWriteAccess>,
    century_register: Option<NonZeroU8>,
}

#[repr(u8)]
enum Register {
    Second = 0x00,
    Minute = 0x02,
    Hour = 0x04,
    Day = 0x07,
    Month = 0x08,
    Year = 0x09,

    StatusA = 0x0A,
    StatusB = 0x0B,

    StatusD = 0x0D,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct StatusA: u8 {
        /// The update in progress (UIP) bit.
        const UIP = 1 << 7;
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct StatusB: u8 {
        /// The data mode (DM) bit.
        ///
        /// This bit is set when the binary format is used; otherwise, the BCD format is used.
        const DM_BINARY = 1 << 2;
        /// The clock mode (CM) bit.
        ///
        /// This bit is set when the 24-hour format is used; otherwise, the 12-hour format is used.
        const CM_24HOUR = 1 << 1;
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct StatusD: u8 {
        /// The valid RAM and time (VRT) bit.
        ///
        /// This bit is set when the RAM and time are valid.
        const VRT = 1 << 7;
    }
}

impl CmosAccess {
    fn read_register(&mut self, reg: Register) -> u8 {
        self.read_register_impl(reg as u8)
    }

    fn read_century(&mut self) -> Option<u8> {
        self.century_register
            .map(|r| self.read_register_impl(r.get()))
    }

    fn read_status_a(&mut self) -> StatusA {
        StatusA::from_bits_truncate(self.read_register_impl(Register::StatusA as u8))
    }

    fn read_status_b(&mut self) -> StatusB {
        StatusB::from_bits_truncate(self.read_register_impl(Register::StatusB as u8))
    }

    fn check_presence(&mut self) -> bool {
        self.read_register_impl(Register::StatusD as u8) == StatusD::VRT.bits()
    }

    fn read_register_impl(&mut self, reg: u8) -> u8 {
        self.io_sel.write(reg);
        self.io_val.read()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CmosData {
    century: Option<NonZeroU8>,
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl CmosData {
    fn read_rtc(rtc: &RtcCmos) -> Self {
        let mut access = rtc.access.lock();

        let mut now = Self::from_rtc_raw(&mut access);

        while let new = Self::from_rtc_raw(&mut access)
            && now != new
        {
            now = new;
        }

        drop(access);

        if !rtc.status_b.contains(StatusB::DM_BINARY) {
            now.convert_bcd_to_binary();
        }
        if !rtc.status_b.contains(StatusB::CM_24HOUR) {
            now.convert_12_hour_to_24_hour();
        }
        now.modify_year();

        now
    }

    fn from_rtc_raw(access: &mut CmosAccess) -> Self {
        while access.read_status_a().contains(StatusA::UIP) {
            core::hint::spin_loop();
        }

        let second = access.read_register(Register::Second);
        let minute = access.read_register(Register::Minute);
        let hour = access.read_register(Register::Hour);
        let day = access.read_register(Register::Day);
        let month = access.read_register(Register::Month);
        let year = access.read_register(Register::Year) as u16;
        let century = access.read_century().and_then(NonZeroU8::new);

        CmosData {
            century,
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Converts BCD values to binary values.
    fn convert_bcd_to_binary(&mut self) {
        fn bcd_to_binary(val: u8) -> u8 {
            (val & 0xF) + (val >> 4) * 10
        }

        self.second = bcd_to_binary(self.second);
        self.minute = bcd_to_binary(self.minute);
        self.hour = bcd_to_binary(self.hour & !Self::HOUR_IS_AFTERNOON)
            | (self.hour & Self::HOUR_IS_AFTERNOON);
        self.day = bcd_to_binary(self.day);
        self.month = bcd_to_binary(self.month);
        self.year = bcd_to_binary(self.year as u8) as u16;
        self.century = self
            .century
            .and_then(|c| NonZeroU8::new(bcd_to_binary(c.get())));
    }

    const HOUR_IS_AFTERNOON: u8 = 0x80;

    /// Converts the 12-hour clock to the 24-hour clock.
    fn convert_12_hour_to_24_hour(&mut self) {
        if self.hour & Self::HOUR_IS_AFTERNOON != 0 {
            self.hour = (self.hour & !Self::HOUR_IS_AFTERNOON) + 12;
        }
    }

    /// Converts the year without the century (e.g., 10) to the year with the century (e.g., 2010).
    fn modify_year(&mut self) {
        const DEFAULT_21_CENTURY: u8 = 20;

        self.year += (self
            .century
            .map(NonZeroU8::get)
            .unwrap_or(DEFAULT_21_CENTURY) as u16)
            * 100;
    }
}
