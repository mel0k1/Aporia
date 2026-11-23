use super::RecvFlags;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecvOutput {
    len: usize,
    flags: RecvFlags,
}

impl RecvOutput {
    pub(in crate::net) const fn new_for_stream(len: usize) -> Self {
        Self {
            len,
            flags: RecvFlags::empty(),
        }
    }

    pub(in crate::net) fn new_for_packet(
        input_flags: RecvFlags,
        copied_len: usize,
        message_len: usize,
    ) -> Self {
        let len = if input_flags.contains(RecvFlags::MSG_TRUNC) {
            message_len
        } else {
            copied_len
        };

        let flags = if copied_len < message_len {
            RecvFlags::MSG_TRUNC
        } else {
            RecvFlags::empty()
        };

        Self { len, flags }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn flags(&self) -> RecvFlags {
        self.flags
    }
}
