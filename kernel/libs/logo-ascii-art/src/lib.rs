#![no_std]

pub fn get_black_white_version() -> &'static str {
    static LOGO_ASCII_ART: &str = include_str!("logo_ascii_art.txt");
    LOGO_ASCII_ART
}

pub fn get_gradient_color_version() -> &'static str {
    static LOGO_ASCII_ART: &str = include_str!("logo_ascii_art_gradient.txt");
    LOGO_ASCII_ART
}
