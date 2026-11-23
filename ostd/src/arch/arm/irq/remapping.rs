pub(crate) struct IrqRemapping {
    _private: (),
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    pub(crate) fn init(&self, _irq_num: u8) {}

    pub(crate) fn remapping_index(&self) -> Option<u16> {
        None
    }
}
