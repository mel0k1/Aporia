use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct GlobalStatus: u32 {
        const CFIS  = 1 << 23;
        const IRTPS = 1 << 24;
        const IRES  = 1 << 25;
        const QIES  = 1 << 26;
        const WBFS  = 1 << 27;
        const RTPS  = 1 << 30;
        const TES   = 1 << 31;
    }
}
