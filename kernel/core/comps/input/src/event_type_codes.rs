use bitflags::bitflags;
use bitvec::prelude::*;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct EventTypes: u32 {
        const SYN       = 1 << 0x00;
        const KEY       = 1 << 0x01;
        const REL       = 1 << 0x02;
        const ABS       = 1 << 0x03;
        const MSC       = 1 << 0x04;
        const SW        = 1 << 0x05;
        const LED       = 1 << 0x11;
        const SND       = 1 << 0x12;
        const REP       = 1 << 0x14;
        const FF        = 1 << 0x15;
        const PWR       = 1 << 0x16;
        const FF_STATUS = 1 << 0x17;
    }
}

impl Default for EventTypes {
    fn default() -> Self {
        Self::new()
    }
}

impl EventTypes {
    pub const fn new() -> Self {
        Self::empty()
    }

    pub const fn as_index(&self) -> u16 {
        let bits = self.bits();
        assert!(
            bits != 0 && (bits & (bits - 1)) == 0,
            "`EventTypes::as_index` expects exactly one event bit to be set"
        );

        match *self {
            Self::SYN => 0x00,
            Self::KEY => 0x01,
            Self::REL => 0x02,
            Self::ABS => 0x03,
            Self::MSC => 0x04,
            Self::SW => 0x05,
            Self::LED => 0x11,
            Self::SND => 0x12,
            Self::REP => 0x14,
            Self::FF => 0x15,
            Self::PWR => 0x16,
            Self::FF_STATUS => 0x17,
            _ => unreachable!(),
        }
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynEvent {
    Report = 0x00,
    Config = 0x01,
    MtReport = 0x02,
    Dropped = 0x03,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelCode {
    X = 0x00,
    Y = 0x01,
    Z = 0x02,
    Rx = 0x03,
    Ry = 0x04,
    Rz = 0x05,
    HWheel = 0x06,
    Dial = 0x07,
    Wheel = 0x08,
    Misc = 0x09,
    Reserved = 0x0a,
    WheelHiRes = 0x0b,
    HWheelHiRes = 0x0c,
}
const REL_MAX: usize = 0x0f;
const REL_COUNT: usize = REL_MAX + 1;

#[derive(Clone, Debug)]
pub struct RelCodeSet(BitVec<u8>);

impl Default for RelCodeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl RelCodeSet {
    pub fn new() -> Self {
        Self(BitVec::repeat(false, REL_COUNT))
    }

    pub fn set(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        self.0.set(index, true);
    }

    pub fn clear(&mut self, rel_code: RelCode) {
        let index = rel_code as usize;
        self.0.set(index, false);
    }

    pub fn contain(&self, rel_code: RelCode) -> bool {
        let index = rel_code as usize;
        self.0.get(index).map(|b| *b).unwrap()
    }

    pub fn as_raw_slice(&self) -> &[u8] {
        self.0.as_raw_slice()
    }
}

#[derive(Clone, Debug)]
pub struct KeyCodeSet(BitVec<u8>);

impl Default for KeyCodeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyCodeSet {
    pub fn new() -> Self {
        Self(BitVec::repeat(false, KEY_COUNT))
    }

    pub fn set(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        self.0.set(index, true);
    }

    pub fn clear(&mut self, key_code: KeyCode) {
        let index = key_code as usize;
        self.0.set(index, false);
    }

    pub fn contain(&self, key_code: KeyCode) -> bool {
        let index = key_code as usize;
        self.0.get(index).map(|b| *b).unwrap()
    }

    pub fn contain_any(&self, range: core::ops::Range<usize>) -> bool {
        assert!(range.is_empty() || range.end <= KEY_COUNT);
        range.into_iter().any(|i| *self.0.get(i).unwrap())
    }

    pub fn as_raw_slice(&self) -> &[u8] {
        self.0.as_raw_slice()
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Reserved = 0,

    Esc = 1,
    F1 = 59,
    F2 = 60,
    F3 = 61,
    F4 = 62,
    F5 = 63,
    F6 = 64,
    F7 = 65,
    F8 = 66,
    F9 = 67,
    F10 = 68,
    F11 = 87,
    F12 = 88,

    Num1 = 2,
    Num2 = 3,
    Num3 = 4,
    Num4 = 5,
    Num5 = 6,
    Num6 = 7,
    Num7 = 8,
    Num8 = 9,
    Num9 = 10,
    Num0 = 11,
    Minus = 12,
    Equal = 13,
    Backspace = 14,

    Tab = 15,
    Q = 16,
    W = 17,
    E = 18,
    R = 19,
    T = 20,
    Y = 21,
    U = 22,
    I = 23,
    O = 24,
    P = 25,
    LeftBrace = 26,
    RightBrace = 27,
    Backslash = 43,

    CapsLock = 58,
    A = 30,
    S = 31,
    D = 32,
    F = 33,
    G = 34,
    H = 35,
    J = 36,
    K = 37,
    L = 38,
    Semicolon = 39,
    Apostrophe = 40,
    Enter = 28,

    LeftShift = 42,
    Z = 44,
    X = 45,
    C = 46,
    V = 47,
    B = 48,
    N = 49,
    M = 50,
    Comma = 51,
    Dot = 52,
    Slash = 53,
    RightShift = 54,

    LeftCtrl = 29,
    LeftAlt = 56,
    Space = 57,
    RightAlt = 100,
    RightCtrl = 97,

    Grave = 41,
    LeftMeta = 125,
    RightMeta = 126,
    Menu = 139,

    Up = 103,
    Down = 108,
    Left = 105,
    Right = 106,

    Home = 102,
    End = 107,
    PageUp = 104,
    PageDown = 109,
    Insert = 110,
    Delete = 111,

    NumLock = 69,
    ScrollLock = 70,

    Kp0 = 82,
    Kp1 = 79,
    Kp2 = 80,
    Kp3 = 81,
    Kp4 = 75,
    Kp5 = 76,
    Kp6 = 77,
    Kp7 = 71,
    Kp8 = 72,
    Kp9 = 73,
    KpDot = 83,
    KpPlus = 78,
    KpMinus = 74,
    KpAsterisk = 55,
    KpSlash = 98,
    KpEnter = 96,

    Mute = 113,
    VolumeDown = 114,
    VolumeUp = 115,

    Power = 116,
    Pause = 119,

    BtnMisc = 0x100,

    BtnLeft = 0x110,
    BtnRight = 0x111,
    BtnMiddle = 0x112,
    BtnSide = 0x113,
    BtnExtra = 0x114,
    BtnForward = 0x115,
    BtnBack = 0x116,
}

const KEY_MAX: usize = 0x120;
const KEY_COUNT: usize = KEY_MAX + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyStatus {
    Released,
    Pressed,
}
