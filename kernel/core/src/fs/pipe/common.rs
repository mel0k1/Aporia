use core::{
    num::Wrapping,
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::sync::WaitQueue;

use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{AccessMode, PerOpenFileOps, SettableStatusFlags, StatusFlags},
        utils::{Endpoint, EndpointState},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            PollHandle, Pollable,
            constants::SIGPIPE,
            signals::user::{UserSignal, UserSignalKind},
        },
    },
    util::{
        ioctl::RawIoctl,
        ring_buffer::{ConsumerU8Ext, ProducerU8Ext, RbConsumer, RbProducer, RingBuffer},
    },
};

pub(in crate::fs) struct PipeHandle {
    inner: Arc<PipeObj>,
    access_mode: AccessMode,
}

impl PipeHandle {
    fn new(inner: Arc<PipeObj>, access_mode: AccessMode) -> Box<Self> {
        Box::new(Self { inner, access_mode })
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        debug_assert!(self.access_mode.is_readable());

        self.inner.reader.try_read(writer)
    }

    fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        debug_assert!(self.access_mode.is_writable());

        self.inner.writer.try_write(reader)
    }

    fn bytes_to_read(&self) -> usize {
        self.inner.reader.buffer_len()
    }
}

impl Pollable for PipeHandle {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        let mut events = IoEvents::empty();

        if self.access_mode.is_readable() {
            events |= self.inner.reader.poll(mask, poller.as_deref_mut());
        }

        if self.access_mode.is_writable() {
            events |= self.inner.writer.poll(mask, poller);
        }

        events
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        if self.access_mode.is_readable() {
            let old_value = self.inner.num_reader.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                self.inner.reader.peer_shutdown();
            }
        }

        if self.access_mode.is_writable() {
            let old_value = self.inner.num_writer.fetch_sub(1, Ordering::Relaxed);
            if old_value == 1 {
                self.inner.writer.shutdown();
            }
        }
    }
}

impl FileOps for PipeHandle {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if !writer.has_avail() {
            return Ok(0);
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_write(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.try_write(reader))
        }
    }
}

impl PerOpenFileOps for PipeHandle {
    fn check_seekable(&self) -> Result<()> {
        Err(errno!(ESPIPE, "the inode is a FIFO file"))
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        use crate::util::ioctl::common_defs::GetNumBytesToRead;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetNumBytesToRead => {
                let bytes_to_read = self.bytes_to_read() as i32;

                cmd.write(&bytes_to_read)?;
                Ok(0)
            }
            _ => Err(errno!(ENOTTY, "the ioctl command is unknown")),
        })
    }

    fn settable_status_flags(&self) -> SettableStatusFlags {
        SettableStatusFlags::minimal().with_o_async()
    }
}

pub(in crate::fs) struct Pipe {
    pipe: Mutex<PipeInner>,
    wait_queue: WaitQueue,
}

impl Pipe {
    pub(crate) fn new() -> Self {
        Self {
            pipe: Mutex::new(PipeInner::default()),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(crate) fn open_named(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        self.open_handle(access_mode, status_flags, true)
    }

    pub(super) fn open_anon(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        self.open_handle(access_mode, status_flags, false)
    }

    fn open_handle(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
        is_named_pipe: bool,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        check_status_flags(status_flags)?;

        let mut pipe = self.pipe.lock();
        let pipe_obj = pipe.get_or_create_pipe_obj();

        let handle = match access_mode {
            AccessMode::O_RDONLY => {
                pipe.read_count += 1;

                let old_value = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                if old_value == 0 {
                    pipe_obj.reader.peer_activate();
                    self.wait_queue.wake_all();
                }

                let has_writer = pipe_obj.num_writer.load(Ordering::Relaxed) > 0;
                let handle = PipeHandle::new(pipe_obj, access_mode);

                if is_named_pipe && !status_flags.contains(StatusFlags::O_NONBLOCK) && !has_writer {
                    let old_write_count = pipe.write_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_write_count != self.pipe.lock().write_count).then_some(())
                    })?;
                }

                handle
            }
            AccessMode::O_WRONLY => {
                pipe.write_count += 1;

                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_writer == 0 {
                    pipe_obj.writer.activate();
                    self.wait_queue.wake_all();
                }

                let has_reader = pipe_obj.num_reader.load(Ordering::Relaxed) > 0;
                let handle = PipeHandle::new(pipe_obj, access_mode);

                if is_named_pipe && !has_reader {
                    if status_flags.contains(StatusFlags::O_NONBLOCK) {
                        return Err(errno!(ENXIO, "no reader is present"));
                    }

                    let old_read_count = pipe.read_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_read_count != self.pipe.lock().read_count).then_some(())
                    })?;
                }

                handle
            }
            AccessMode::O_RDWR => {
                pipe.read_count += 1;
                pipe.write_count += 1;

                let old_num_reader = pipe_obj.num_reader.fetch_add(1, Ordering::Relaxed);
                let old_num_writer = pipe_obj.num_writer.fetch_add(1, Ordering::Relaxed);
                if old_num_reader == 0 || old_num_writer == 0 {
                    self.wait_queue.wake_all();
                    pipe_obj.writer.activate();
                }

                PipeHandle::new(pipe_obj, access_mode)
            }
        };

        Ok(handle)
    }
}

pub(in crate::fs) fn check_status_flags(status_flags: StatusFlags) -> Result<()> {
    if status_flags.contains(StatusFlags::O_DIRECT) {
        return Err(errno!(EINVAL, "the `O_DIRECT` flag is not supported"));
    }

    Ok(())
}

struct PipeObj {
    reader: PipeReader,
    writer: PipeWriter,
    num_reader: AtomicUsize,
    num_writer: AtomicUsize,
}

impl PipeObj {
    fn new() -> Arc<Self> {
        let (reader, writer) = new_pair();
        Arc::new(Self {
            reader,
            writer,
            num_reader: AtomicUsize::new(0),
            num_writer: AtomicUsize::new(0),
        })
    }
}

#[derive(Default)]
struct PipeInner {
    pipe_obj: Weak<PipeObj>,
    read_count: Wrapping<usize>,
    write_count: Wrapping<usize>,
}

impl PipeInner {
    fn get_or_create_pipe_obj(&mut self) -> Arc<PipeObj> {
        if let Some(pipe_obj) = self.pipe_obj.upgrade() {
            return pipe_obj;
        }

        let pipe_obj = PipeObj::new();
        self.pipe_obj = Arc::downgrade(&pipe_obj);

        pipe_obj
    }
}

#[cfg(not(ktest))]
const DEFAULT_PIPE_BUF_SIZE: usize = 65536;
#[cfg(ktest)]
const DEFAULT_PIPE_BUF_SIZE: usize = 2;

#[cfg(not(ktest))]
const PIPE_BUF: usize = 4096;
#[cfg(ktest)]
const PIPE_BUF: usize = 2;

fn new_pair() -> (PipeReader, PipeWriter) {
    new_pair_with_capacity(DEFAULT_PIPE_BUF_SIZE)
}

fn new_pair_with_capacity(capacity: usize) -> (PipeReader, PipeWriter) {
    let (producer, consumer) = RingBuffer::new(capacity).split();
    let (producer_state, consumer_state) =
        Endpoint::new_pair(EndpointState::default(), EndpointState::default());

    (
        PipeReader::new(consumer, consumer_state),
        PipeWriter::new(producer, producer_state),
    )
}

struct PipeReader {
    consumer: Mutex<RbConsumer<u8>>,
    state: Endpoint<EndpointState>,
}

impl PipeReader {
    fn new(consumer: RbConsumer<u8>, state: Endpoint<EndpointState>) -> Self {
        Self {
            consumer: Mutex::new(consumer),
            state,
        }
    }

    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let read = || {
            let mut consumer = self.consumer.lock();
            consumer.read_fallible(writer)
        };

        let read_len = self.state.read_with(read)?;
        if read_len > 0 {
            self.state.notify_read();
        }

        Ok(read_len)
    }

    fn buffer_len(&self) -> usize {
        self.consumer.lock().len()
    }

    fn peer_shutdown(&self) {
        self.state.peer_shutdown();
    }

    fn peer_activate(&self) {
        self.state.peer_activate();
    }

    fn check_io_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();
        if self.state.is_peer_shutdown() {
            events |= IoEvents::HUP;
        }
        if !self.consumer.lock().is_empty() {
            events |= IoEvents::IN;
        }
        events
    }
}

impl Pollable for PipeReader {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.state
            .poll_with(mask, poller, || self.check_io_events())
    }
}

struct PipeWriter {
    producer: Mutex<RbProducer<u8>>,
    state: Endpoint<EndpointState>,
}

impl PipeWriter {
    fn new(producer: RbProducer<u8>, state: Endpoint<EndpointState>) -> Self {
        Self {
            producer: Mutex::new(producer),
            state,
        }
    }

    fn try_write(&self, reader: &mut VmReader) -> Result<usize> {
        let write = || {
            let mut producer = self.producer.lock();
            if reader.remain() <= PIPE_BUF && producer.free_len() < reader.remain() {
                return Ok(0);
            }
            producer.write_fallible(reader)
        };

        let res = self.state.write_with(write);
        if res.is_ok() {
            self.state.notify_write();
        }
        if res.is_err_and(|e| e.error() == Errno::EPIPE)
            && let Some(posix_thread) = current_thread!().as_posix_thread()
        {
            posix_thread.enqueue_signal(Box::new(UserSignal::new(
                SIGPIPE,
                UserSignalKind::Kill,
                posix_thread.process().pid(),
                posix_thread.credentials().ruid(),
            )));
        }

        res
    }

    fn shutdown(&self) {
        self.state.shutdown();
    }

    fn activate(&self) {
        self.state.activate();
    }

    fn check_io_events(&self) -> IoEvents {
        if self.state.is_shutdown() {
            IoEvents::ERR | IoEvents::OUT
        } else if self.producer.lock().free_len() >= PIPE_BUF {
            IoEvents::OUT
        } else {
            IoEvents::empty()
        }
    }
}

impl Pollable for PipeWriter {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.state
            .poll_with(mask, poller, || self.check_io_events())
    }
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::{self, AtomicBool};

    use ostd::prelude::*;

    use super::*;
    use crate::{
        prelude::Result,
        thread::{Thread, kernel_thread::ThreadOptions},
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Ordering {
        WriteThenRead,
        ReadThenWrite,
    }

    fn test_blocking<W, R>(write: W, read: R, ordering: Ordering)
    where
        W: FnOnce(Box<dyn PerOpenFileOps>) + Send + 'static,
        R: FnOnce(Box<dyn PerOpenFileOps>) + Send + 'static,
    {
        let pipe = Pipe::new();
        let reader = pipe
            .open_anon(AccessMode::O_RDONLY, StatusFlags::empty())
            .unwrap();
        let writer = pipe
            .open_anon(AccessMode::O_WRONLY, StatusFlags::empty())
            .unwrap();

        let signal_writer = Arc::new(AtomicBool::new(false));
        let signal_reader = signal_writer.clone();

        let writer = ThreadOptions::new(move || {
            if ordering == Ordering::ReadThenWrite {
                while !signal_writer.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_writer.store(true, atomic::Ordering::Relaxed);
            }

            write(writer);
        })
        .spawn();

        let reader = ThreadOptions::new(move || {
            if ordering == Ordering::WriteThenRead {
                while !signal_reader.load(atomic::Ordering::Relaxed) {
                    Thread::yield_now();
                }
            } else {
                signal_reader.store(true, atomic::Ordering::Relaxed);
            }

            read(reader);
        })
        .spawn();

        writer.join();
        reader.join();
    }

    #[ktest]
    fn read_empty() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1]).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf, &[1]);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn write_full() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1, 2, 3]).unwrap(), 2);
                assert_eq!(write(writer.as_ref(), &[2]).unwrap(), 1);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[2]);
            },
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn read_closed() {
        test_blocking(
            drop,
            |reader| {
                let mut buf = [0; 1];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 0);
            },
            Ordering::ReadThenWrite,
        );
    }

    #[ktest]
    fn write_closed() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1, 2, 3]).unwrap(), 2);
                assert_eq!(
                    write(writer.as_ref(), &[2]).unwrap_err().error(),
                    Errno::EPIPE
                );
            },
            drop,
            Ordering::WriteThenRead,
        );
    }

    #[ktest]
    fn write_atomicity() {
        test_blocking(
            |writer| {
                assert_eq!(write(writer.as_ref(), &[1]).unwrap(), 1);
                assert_eq!(write(writer.as_ref(), &[1, 2]).unwrap(), 2);
            },
            |reader| {
                let mut buf = [0; 3];
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 1);
                assert_eq!(&buf[..1], &[1]);
                assert_eq!(read(reader.as_ref(), &mut buf).unwrap(), 2);
                assert_eq!(&buf[..2], &[1, 2]);
            },
            Ordering::WriteThenRead,
        );
    }

    fn read(reader: &dyn PerOpenFileOps, buf: &mut [u8]) -> Result<usize> {
        reader.read_at(
            0,
            &mut VmWriter::from(buf).to_fallible(),
            StatusFlags::empty(),
        )
    }

    fn write(writer: &dyn PerOpenFileOps, buf: &[u8]) -> Result<usize> {
        writer.write_at(
            0,
            &mut VmReader::from(buf).to_fallible(),
            StatusFlags::empty(),
        )
    }
}
