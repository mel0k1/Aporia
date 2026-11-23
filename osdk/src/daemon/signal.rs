use std::{
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    thread::JoinHandle,
};

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::{Handle as SignalHandle, Signals},
    low_level::emulate_default_handler,
};

use crate::{error::Errno, error_msg};

pub(crate) struct SignalGuard {
    signal: Arc<AtomicI32>,
    handle: SignalHandle,
    thread: Option<JoinHandle<()>>,
}

impl SignalGuard {
    pub(crate) fn install() -> Result<Self, Errno> {
        let mut mask = Signals::new([SIGINT, SIGTERM, SIGHUP]).map_err(|err| {
            error_msg!("failed to register signal handlers: {err}");
            Errno::ExecuteCommand
        })?;
        let signal = Arc::new(AtomicI32::new(0));
        let signal_for_thread = signal.clone();
        let handle = mask.handle();

        let thread = std::thread::spawn(move || {
            let mut first_signal = true;
            for received_signal in mask.forever() {
                if !first_signal {
                    let _ = emulate_default_handler(received_signal);
                    continue;
                }
                signal_for_thread.store(received_signal, Ordering::SeqCst);
                first_signal = false;
            }
        });

        Ok(Self {
            signal,
            handle,
            thread: Some(thread),
        })
    }

    pub(crate) fn received_signal(&self) -> Option<i32> {
        match self.signal.load(Ordering::SeqCst) {
            0 => None,
            signal => Some(signal),
        }
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
