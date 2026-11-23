mod handler;
mod keysym;

use core::sync::atomic::{AtomicU8, Ordering};

use aster_framebuffer::mode::{KeyboardMode, KeyboardModeFlags};

use crate::prelude::*;

pub(super) fn init_in_first_process() {
    keysym::init_in_first_process();
    handler::init_in_first_process();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModifierKey {
    Shift,
    Ctrl,
    Alt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumpadKey {
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Dot,
    Enter,
    Plus,
    Minus,
    Asterisk,
    Slash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CursorKey {
    Up,
    Down,
    Left,
    Right,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct ModifierKeyFlags: u8 {
        const SHIFT = 1 << 0;
        const ALT   = 1 << 1;
        const CTRL  = 1 << 2;
    }

    #[derive(Clone, Copy, Debug)]
    struct LockKeyFlags: u8 {
        const CAPS_LOCK   = 1 << 0;
        const NUM_LOCK    = 1 << 1;
        const SCROLL_LOCK = 1 << 2;
    }
}

#[derive(Debug)]
struct ModifierKeysState {
    inner: AtomicU8,
}

impl ModifierKeysState {
    const fn new() -> Self {
        Self {
            inner: AtomicU8::new(0),
        }
    }

    fn press(&self, keys: ModifierKeyFlags) {
        self.inner.fetch_or(keys.bits(), Ordering::Relaxed);
    }

    fn release(&self, keys: ModifierKeyFlags) {
        self.inner.fetch_and(!keys.bits(), Ordering::Relaxed);
    }

    fn flags(&self) -> ModifierKeyFlags {
        ModifierKeyFlags::from_bits_truncate(self.inner.load(Ordering::Relaxed))
    }
}

pub(super) struct VtKeyboard {
    lock_key_flags: LockKeyFlags,
    mode: KeyboardMode,
    mode_flags: KeyboardModeFlags,
}

impl Default for VtKeyboard {
    fn default() -> Self {
        Self {
            lock_key_flags: LockKeyFlags::empty(),
            mode: KeyboardMode::Unicode,
            mode_flags: KeyboardModeFlags::REPEAT | KeyboardModeFlags::META,
        }
    }
}

impl VtKeyboard {
    fn lock_key_flags(&self) -> LockKeyFlags {
        self.lock_key_flags
    }

    fn toggle_lock_keys(&mut self, keys: LockKeyFlags) {
        self.lock_key_flags.toggle(keys);
    }

    pub(super) fn mode(&self) -> KeyboardMode {
        self.mode
    }

    pub(super) fn set_mode(&mut self, mode: KeyboardMode) -> Result<()> {
        match mode {
            KeyboardMode::Raw => Err(errno!(EINVAL, "the keyboard mode is not supported")),
            KeyboardMode::Xlate
            | KeyboardMode::MediumRaw
            | KeyboardMode::Unicode
            | KeyboardMode::Off => {
                self.mode = mode;
                Ok(())
            }
        }
    }

    pub(super) fn mode_flags(&self) -> KeyboardModeFlags {
        self.mode_flags
    }
}
