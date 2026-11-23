use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use inherit_methods_macro::inherit_methods;
use ostd::{
    console::uart_ns16650a::{Ns16550aAccess, Ns16550aUart},
    mm::VmReader,
    sync::{LocalIrqDisabled, SpinLock},
};

pub(super) struct UartConsole<U: Uart> {
    uart: U,
    callbacks: SpinLock<Vec<&'static ConsoleCallback>, LocalIrqDisabled>,
}

impl<U: Uart> Debug for UartConsole<U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UartConsole").finish_non_exhaustive()
    }
}

impl<U: Uart> UartConsole<U> {
    pub(super) fn new(uart: U) -> Arc<Self> {
        Arc::new(Self {
            uart,
            callbacks: SpinLock::new(Vec::new()),
        })
    }

    #[cfg_attr(not(target_arch = "riscv64"), expect(dead_code))]
    pub(super) fn uart(&self) -> &U {
        &self.uart
    }

    pub(super) fn trigger_input_callbacks(&self) {
        let mut buf = [0; 16];

        loop {
            let num_rcv = self.uart.recv(&mut buf);
            if num_rcv == 0 {
                break;
            }

            let reader = VmReader::from(&buf[..num_rcv]);
            for callback in self.callbacks.lock().iter() {
                (callback)(reader.clone());
            }

            if num_rcv < buf.len() {
                break;
            }
        }
    }
}

impl<U: Uart + Send + Sync + 'static> AnyConsoleDevice for UartConsole<U> {
    fn send(&self, buf: &[u8]) {
        self.uart.send(buf);
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        self.callbacks.lock().push(callback);
    }
}

pub(super) trait Uart {
    fn send(&self, buf: &[u8]);

    #[must_use]
    fn recv(&self, buf: &mut [u8]) -> usize;

    fn flush(&self);
}

pub(super) trait UartMut {
    fn send_byte(&mut self, byte: u8);

    fn recv_byte(&mut self) -> Option<u8>;
}

impl<T: UartMut> Uart for SpinLock<T, LocalIrqDisabled> {
    fn send(&self, buf: &[u8]) {
        let mut uart = self.lock();

        for byte in buf {
            if *byte == b'\n' {
                uart.send_byte(b'\r');
            }
            uart.send_byte(*byte);
        }
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        let mut uart = self.lock();

        for (i, byte) in buf.iter_mut().enumerate() {
            let Some(recv_byte) = uart.recv_byte() else {
                return i;
            };
            *byte = recv_byte;
        }

        buf.len()
    }

    fn flush(&self) {
        let mut uart = self.lock();

        while uart.recv_byte().is_some() {}
    }
}

#[inherit_methods(from = "(**self)")]
impl<T: UartMut> Uart for &SpinLock<T, LocalIrqDisabled> {
    fn send(&self, buf: &[u8]);
    fn recv(&self, buf: &mut [u8]) -> usize;
    fn flush(&self);
}

impl<A: Ns16550aAccess> UartMut for Ns16550aUart<A> {
    fn send_byte(&mut self, byte: u8) {
        self.send(byte);
    }

    fn recv_byte(&mut self) -> Option<u8> {
        self.recv()
    }
}
