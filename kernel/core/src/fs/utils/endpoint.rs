use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

pub(crate) struct Endpoint<T> {
    inner: Arc<Inner<T>>,
    location: Location,
}

enum Location {
    Client,
    Server,
}

struct Inner<T> {
    client: T,
    server: T,
}

impl<T> Endpoint<T> {
    pub(crate) fn new_pair(this: T, peer: T) -> (Endpoint<T>, Endpoint<T>) {
        let inner = Arc::new(Inner {
            client: this,
            server: peer,
        });

        let client = Endpoint {
            inner: inner.clone(),
            location: Location::Client,
        };
        let server = Endpoint {
            inner,
            location: Location::Server,
        };

        (client, server)
    }

    pub(crate) fn this_end(&self) -> &T {
        match self.location {
            Location::Client => &self.inner.client,
            Location::Server => &self.inner.server,
        }
    }

    pub(crate) fn peer_end(&self) -> &T {
        match self.location {
            Location::Client => &self.inner.server,
            Location::Server => &self.inner.client,
        }
    }
}

pub(crate) struct EndpointState {
    pollee: Pollee,
    is_shutdown: AtomicBool,
}

impl EndpointState {
    pub(crate) fn new(pollee: Pollee, is_shutdown: bool) -> Self {
        Self {
            pollee,
            is_shutdown: AtomicBool::new(is_shutdown),
        }
    }

    pub(crate) fn cloned_pollee(&self) -> Pollee {
        self.pollee.clone()
    }
}

impl Default for EndpointState {
    fn default() -> Self {
        Self::new(Pollee::new(), false)
    }
}

impl AsRef<EndpointState> for EndpointState {
    fn as_ref(&self) -> &EndpointState {
        self
    }
}

impl<T: AsRef<EndpointState>> Endpoint<T> {
    pub(crate) fn read_with<F>(&self, read: F) -> Result<usize>
    where
        F: FnOnce() -> Result<usize>,
    {
        let is_shutdown = self.is_peer_shutdown();

        let read_len = read()?;

        if read_len > 0 {
            Ok(read_len)
        } else if is_shutdown {
            Ok(0)
        } else {
            Err(errno!(EAGAIN, "the channel is empty"))
        }
    }

    pub(crate) fn write_with<F>(&self, write: F) -> Result<usize>
    where
        F: FnOnce() -> Result<usize>,
    {
        if self.is_shutdown() {
            return Err(errno!(EPIPE, "the channel is shut down"));
        }

        let written_len = write()?;

        if written_len == 0 {
            return Err(errno!(EAGAIN, "the channel is full"));
        }

        Ok(written_len)
    }

    pub(crate) fn notify_read(&self) {
        self.peer_end().as_ref().pollee.notify(IoEvents::OUT);
        self.this_end().as_ref().pollee.invalidate();
    }

    pub(crate) fn notify_write(&self) {
        self.peer_end().as_ref().pollee.notify(IoEvents::IN);
        self.this_end().as_ref().pollee.invalidate();
    }

    pub(crate) fn poll_with<F>(
        &self,
        mask: IoEvents,
        poller: Option<&mut PollHandle>,
        check: F,
    ) -> IoEvents
    where
        F: FnOnce() -> IoEvents,
    {
        self.this_end()
            .as_ref()
            .pollee
            .poll_with(mask, poller, check)
    }

    pub(crate) fn shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(this_end, peer_end);
    }

    pub(crate) fn peer_shutdown(&self) {
        let this_end = self.this_end().as_ref();
        let peer_end = self.peer_end().as_ref();

        Self::shutdown_impl(peer_end, this_end);
    }

    fn shutdown_impl(this_end: &EndpointState, peer_end: &EndpointState) {
        this_end.is_shutdown.store(true, Ordering::Relaxed);
        peer_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::RDHUP | IoEvents::IN);
        this_end
            .pollee
            .notify(IoEvents::HUP | IoEvents::ERR | IoEvents::OUT);
    }

    pub(in crate::fs) fn activate(&self) {
        self.this_end()
            .as_ref()
            .is_shutdown
            .store(false, Ordering::Relaxed);
        self.this_end().as_ref().pollee.invalidate();
    }

    pub(in crate::fs) fn peer_activate(&self) {
        self.peer_end()
            .as_ref()
            .is_shutdown
            .store(false, Ordering::Relaxed);
        self.peer_end().as_ref().pollee.invalidate();
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.this_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }

    pub(crate) fn is_peer_shutdown(&self) -> bool {
        self.peer_end().as_ref().is_shutdown.load(Ordering::Relaxed)
    }
}
