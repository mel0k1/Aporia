use core::fmt;

use bitflags::bitflags;

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub enum Ns16550aRegister {
    DataOrDivisorLo,
    IntEnOrDivisorHi,
    FifoCtrl,
    LineCtrl,
    ModemCtrl,
    LineStat,
    ModemStat,
}

impl Ns16550aRegister {
    pub const MAX: Self = Self::ModemStat;
}

pub trait Ns16550aAccess {
    fn read(&self, reg: Ns16550aRegister) -> u8;

    fn write(&mut self, reg: Ns16550aRegister, val: u8);
}

#[derive(Debug)]
pub struct Ns16550aUart<A: Ns16550aAccess> {
    access: A,
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct LineStat: u8 {
        const DR   = 1 << 0;
        const THRE = 1 << 5;
    }
}

impl<A: Ns16550aAccess> Ns16550aUart<A> {
    pub const fn new(access: A) -> Self {
        Self { access }
    }

    pub fn init(&mut self) {
        const DLAB: u8 = 0x80;

        self.access.write(Ns16550aRegister::LineCtrl, DLAB);
        self.access.write(Ns16550aRegister::DataOrDivisorLo, 0x01);
        self.access.write(Ns16550aRegister::IntEnOrDivisorHi, 0x00);

        self.access.write(Ns16550aRegister::LineCtrl, 0x03);
        self.access.write(Ns16550aRegister::FifoCtrl, 0x00);
        self.access.write(Ns16550aRegister::ModemCtrl, 0x0B);
        self.access.write(Ns16550aRegister::IntEnOrDivisorHi, 0x01);
    }

    pub fn send(&mut self, data: u8) {
        while !self.line_stat().contains(LineStat::THRE) {
            core::hint::spin_loop();
        }

        self.access.write(Ns16550aRegister::DataOrDivisorLo, data);
    }

    pub fn recv(&mut self) -> Option<u8> {
        if !self.line_stat().contains(LineStat::DR) {
            return None;
        }

        Some(self.access.read(Ns16550aRegister::DataOrDivisorLo))
    }

    fn line_stat(&self) -> LineStat {
        LineStat::from_bits_truncate(self.access.read(Ns16550aRegister::LineStat))
    }
}

impl<A: Ns16550aAccess> fmt::Write for Ns16550aUart<A> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.as_bytes() {
            if *c == b'\n' {
                self.send(b'\r');
            }
            self.send(*c);
        }
        Ok(())
    }
}
