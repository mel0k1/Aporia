use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct GlobalCommand: u32 {
        const CFI   = 1 << 23;
        const SIRTP = 1 << 24;
        const IRE   = 1 << 25;
        const QIE   = 1 << 26;
        const WBF   = 1 << 27;
        const SRTP  = 1 << 30;
        const TE    = 1 << 31;
    }
}
