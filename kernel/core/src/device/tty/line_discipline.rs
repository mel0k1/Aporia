use ostd::const_assert;

use super::termio::{CCtrlCharId, CTermios, CTermios2, CTermiosSpeeds, CWinSize};
use crate::{
    device::tty::termio::{CInputFlags, CLocalFlags},
    prelude::*,
    process::signal::{
        constants::{SIGINT, SIGQUIT, SIGTSTP},
        sig_num::SigNum,
    },
    util::ring_buffer::RingBuffer,
};

const LINE_CAPACITY: usize = 4095;
const BUFFER_CAPACITY: usize = 8192;

const_assert!(LINE_CAPACITY < BUFFER_CAPACITY);

pub(crate) struct LineDiscipline {
    current_line: CurrentLine,
    read_buffer: RingBuffer<u8>,
    termios: CTermios2,
    winsize: CWinSize,
}

struct CurrentLine {
    buffer: Box<[u8]>,
    len: usize,
}

impl Default for CurrentLine {
    fn default() -> Self {
        Self {
            buffer: vec![0; LINE_CAPACITY].into_boxed_slice(),
            len: 0,
        }
    }
}

impl CurrentLine {
    fn push_char(&mut self, ch: u8) {
        if self.len == self.buffer.len() {
            return;
        }

        self.buffer[self.len] = ch;
        self.len += 1;
    }

    fn drain(&mut self) -> &[u8] {
        let chs = &self.buffer[..self.len];
        self.len = 0;
        chs
    }

    fn backspace(&mut self) {
        self.len = self.len.saturating_sub(1);
    }

    fn len(&self) -> usize {
        self.len
    }
}

impl LineDiscipline {
    pub(crate) fn new() -> Self {
        Self {
            current_line: CurrentLine::default(),
            read_buffer: RingBuffer::new(BUFFER_CAPACITY),
            termios: CTermios2::default(),
            winsize: CWinSize::default(),
        }
    }

    pub(crate) fn push_char<F1: FnMut(SigNum), F2: FnMut(&[u8])>(
        &mut self,
        ch: u8,
        mut signal_callback: F1,
        echo_callback: F2,
    ) -> Result<()> {
        let ch = if self.termios.input_flags().contains(CInputFlags::ICRNL) && ch == b'\r' {
            b'\n'
        } else {
            ch
        };

        if let Some(signum) = char_to_signal(ch, &self.termios) {
            signal_callback(signum);

            if !self.termios.local_flags().contains(CLocalFlags::NOFLSH) {
                self.drain_input();
            }

            if self.termios.local_flags().contains(CLocalFlags::ECHO) {
                self.output_char(ch, echo_callback);
            }

            return Ok(());
        }

        if self.termios.local_flags().contains(CLocalFlags::ECHO) {
            self.output_char(ch, echo_callback);
        }

        if self.is_full() {
            return Err(errno!(EAGAIN, "the buffer is full"));
        }

        if !self.termios.is_canonical_mode() {
            self.read_buffer.push(ch).unwrap();
            return Ok(());
        }

        if ch == self.termios.special_char(CCtrlCharId::VKILL) {
            self.current_line.drain();
        }

        if ch == self.termios.special_char(CCtrlCharId::VERASE) {
            self.current_line.backspace();
        }

        if is_line_terminator(ch, &self.termios) {
            self.flush_line().unwrap();
            self.read_buffer.push(ch).unwrap();
        }

        if is_printable_char(ch) {
            self.current_line.push_char(ch);
        }

        Ok(())
    }

    fn output_char<F: FnMut(&[u8])>(&self, ch: u8, mut echo_callback: F) {
        match ch {
            b'\n' => echo_callback(b"\n"),
            b'\r' => echo_callback(b"\r\n"),
            ch if ch == self.termios.special_char(CCtrlCharId::VERASE) => {
                echo_callback(b"\x08");
            }
            ch if is_printable_char(ch) => echo_callback(&[ch]),
            ch if is_ctrl_char(ch) && self.termios.local_flags().contains(CLocalFlags::ECHOCTL) => {
                echo_callback(&[b'^', ctrl_char_to_printable(ch)]);
            }
            _ => {}
        }
    }

    pub(crate) fn try_read(&mut self, dst: &mut [u8]) -> Result<usize> {
        let vmin = self.termios.special_char(CCtrlCharId::VMIN);
        let vtime = self.termios.special_char(CCtrlCharId::VTIME);

        if vtime != 0 {
            warn!("non-zero VTIME is not supported");
        }

        if self.buffer_len() < dst.len().min(vmin as _) {
            return Err(errno!(
                EAGAIN,
                "the characters in the buffer are not enough"
            ));
        }

        for (i, dst_i) in dst.iter_mut().enumerate() {
            let Some(ch) = self.read_buffer.pop() else {
                return Ok(i);
            };

            if self.termios.is_canonical_mode() && is_eof(ch, &self.termios) {
                return Ok(i);
            }

            *dst_i = ch;

            if self.termios.is_canonical_mode() && is_line_terminator(ch, &self.termios) {
                return Ok(i + 1);
            }
        }

        Ok(dst.len())
    }

    pub(crate) fn drain_input(&mut self) {
        self.current_line.drain();
        self.read_buffer.clear();
    }

    pub(crate) fn buffer_len(&self) -> usize {
        self.read_buffer.len()
    }

    pub(crate) fn is_full(&self) -> bool {
        self.read_buffer.len() + self.current_line.len() >= self.read_buffer.capacity()
    }

    pub(crate) fn termios(&self) -> &CTermios2 {
        &self.termios
    }

    pub(crate) fn set_termios(&mut self, termios: CTermios) {
        let termios2 = CTermios2::new(
            termios,
            CTermiosSpeeds::from_termios(&termios, *self.termios.speeds()),
        );
        self.set_termios2(termios2);
    }

    pub(crate) fn set_termios2(&mut self, termios2: CTermios2) {
        self.termios = termios2;

        if !self.termios.is_canonical_mode() {
            self.flush_line().unwrap();
        }
    }

    fn flush_line(&mut self) -> Option<()> {
        let bytes = self.current_line.drain();
        self.read_buffer.push_slice(bytes)
    }

    pub(crate) fn window_size(&self) -> CWinSize {
        self.winsize
    }

    pub(crate) fn set_window_size(&mut self, winsize: CWinSize) {
        self.winsize = winsize;
    }
}

fn is_line_terminator(ch: u8, termios: &CTermios) -> bool {
    if ch == b'\n'
        || ch == termios.special_char(CCtrlCharId::VEOF)
        || ch == termios.special_char(CCtrlCharId::VEOL)
    {
        return true;
    }

    if termios.local_flags().contains(CLocalFlags::IEXTEN)
        && ch == termios.special_char(CCtrlCharId::VEOL2)
    {
        return true;
    }

    false
}

fn is_eof(ch: u8, termios: &CTermios) -> bool {
    ch == termios.special_char(CCtrlCharId::VEOF)
}

fn is_printable_char(ch: u8) -> bool {
    (0x20..0x7f).contains(&ch)
}

fn is_ctrl_char(ch: u8) -> bool {
    if ch == b'\r' || ch == b'\n' {
        return false;
    }

    (0..0x20).contains(&ch)
}

fn char_to_signal(ch: u8, termios: &CTermios) -> Option<SigNum> {
    if ch == b'\0' || !termios.local_flags().contains(CLocalFlags::ISIG) {
        return None;
    }

    match ch {
        ch if ch == termios.special_char(CCtrlCharId::VINTR) => Some(SIGINT),
        ch if ch == termios.special_char(CCtrlCharId::VQUIT) => Some(SIGQUIT),
        ch if ch == termios.special_char(CCtrlCharId::VSUSP) => Some(SIGTSTP),
        _ => None,
    }
}

fn ctrl_char_to_printable(ch: u8) -> u8 {
    debug_assert!(is_ctrl_char(ch));
    ch + b'A' - 1
}
