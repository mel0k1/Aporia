use super::signal::sig_num::SigNum;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TermStatus {
    Exited(u8),
    Killed(SigNum),
}

impl TermStatus {
    pub(crate) fn as_u32(&self) -> u32 {
        match self {
            TermStatus::Exited(status) => (*status as u32) << 8,
            TermStatus::Killed(signum) => signum.as_u8() as u32,
        }
    }
}
