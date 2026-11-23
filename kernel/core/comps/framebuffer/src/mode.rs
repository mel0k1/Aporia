use int_to_c_enum::TryFromInt;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum ConsoleMode {
    Text = 0,
    Graphics = 1,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum KeyboardMode {
    Raw = 0,
    Xlate = 1,
    MediumRaw = 2,
    Unicode = 3,
    Off = 4,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct KeyboardModeFlags: u8 {
        const APPLICATION = 1 << 0;
        const CURSOR_KEY  = 1 << 1;
        const REPEAT      = 1 << 2;
        const CRLF        = 1 << 3;
        const META        = 1 << 4;
    }
}
