use crate::power::ExitCode;

pub fn try_poweroff(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::Shutdown, sbi_rt::SystemFailure),
    };
}

pub fn try_restart(code: ExitCode) {
    let _ = match code {
        ExitCode::Success => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::NoReason),
        ExitCode::Failure => sbi_rt::system_reset(sbi_rt::ColdReboot, sbi_rt::SystemFailure),
    };
}
