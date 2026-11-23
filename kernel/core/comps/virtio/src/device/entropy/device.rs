use alloc::{boxed::Box, format, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::{
    Error,
    arch::trap::TrapFrame,
    mm::{
        Fallible, FallibleVmRead, PAGE_SIZE, VmWriter,
        dma::{DmaStream, FromDevice},
        io::util::HasVmReaderWriter,
    },
    sync::{LocalIrqDisabled, Mutex, SpinLock, WaitQueue},
};

use crate::{
    device::{VirtioDeviceError, entropy},
    queue::VirtQueue,
    transport::DeviceTransport,
};

static ENTROPY_DEVICE_ID: AtomicUsize = AtomicUsize::new(0);

const ENTROPY_DEVICE_PREFIX: &str = "virtio_rng.";

const ENTROPY_QUEUE_SIZE: u16 = 1;

const ENTROPY_BUFFER_SIZE: usize = PAGE_SIZE;

pub struct EntropyDevice {
    transport: SpinLock<DeviceTransport>,
    inner: SpinLock<EntropyDeviceInner, LocalIrqDisabled>,
    cache: Mutex<EntropyCache>,
    wait_queue: WaitQueue,
}

impl EntropyDevice {
    pub(crate) fn init(mut device_transport: DeviceTransport) -> Result<(), VirtioDeviceError> {
        let queue = VirtQueue::new(0, ENTROPY_QUEUE_SIZE, device_transport.as_mut())?;
        let inner = EntropyDeviceInner::new(queue)?;
        let cache = EntropyCache::new()?;
        let device = Arc::new(EntropyDevice {
            transport: SpinLock::new(device_transport),
            inner: SpinLock::new(inner),
            cache: Mutex::new(cache),
            wait_queue: WaitQueue::new(),
        });

        let mut transport = device.transport.lock();

        transport.register_queue_callback(
            0,
            Box::new({
                let device = Arc::downgrade(&device);
                move |_: &TrapFrame| {
                    if let Some(device) = device.upgrade() {
                        device.handle_recv_irq()
                    }
                }
            }),
            false,
        )?;

        transport.finish_init();
        drop(transport);

        let device_id = ENTROPY_DEVICE_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{ENTROPY_DEVICE_PREFIX}{device_id}");

        entropy::register_device(name, device);

        Ok(())
    }

    pub fn try_read(&self, writer: &mut VmWriter<'_, Fallible>) -> Result<Option<usize>, Error> {
        let mut cache = self.cache.lock();

        if !cache.is_empty() {
            return Ok(Some(cache.drain_into(writer)?));
        }

        {
            let mut inner = self.inner.lock();
            if inner.ready_len > 0 {
                let len = core::mem::replace(&mut inner.ready_len, 0);
                cache.swap_in(&mut inner.dma_buf, len);
            } else {
                if !inner.in_flight {
                    inner.submit();
                }
                return Ok(None);
            }
        }

        Ok(Some(cache.drain_into(writer)?))
    }

    pub fn wait_queue(&self) -> &WaitQueue {
        &self.wait_queue
    }

    fn handle_recv_irq(&self) {
        let mut inner = self.inner.lock();

        let Ok((_, used_len)) = inner.queue.pop_used() else {
            return;
        };

        inner.ready_len = used_len as usize;
        inner.in_flight = false;

        drop(inner);

        self.wait_queue.wake_all();
    }
}

struct EntropyDeviceInner {
    queue: VirtQueue,
    dma_buf: DmaStream<FromDevice>,
    ready_len: usize,
    in_flight: bool,
}

impl EntropyDeviceInner {
    fn new(queue: VirtQueue) -> Result<Self, VirtioDeviceError> {
        let dma_buf = DmaStream::<FromDevice>::alloc_uninit(ENTROPY_BUFFER_SIZE / PAGE_SIZE, false)
            .map_err(VirtioDeviceError::ResourceAlloc)?;
        Ok(Self {
            queue,
            dma_buf,
            ready_len: 0,
            in_flight: false,
        })
    }

    fn submit(&mut self) {
        let Self {
            queue,
            dma_buf,
            in_flight,
            ..
        } = self;

        queue.add_output_bufs(&[dma_buf]).unwrap();
        if queue.should_notify() {
            queue.notify();
        }
        *in_flight = true;
    }
}

struct EntropyCache {
    dma_buf: DmaStream<FromDevice>,
    avail: usize,
}

impl EntropyCache {
    fn new() -> Result<Self, VirtioDeviceError> {
        let dma_buf = DmaStream::<FromDevice>::alloc_uninit(ENTROPY_BUFFER_SIZE / PAGE_SIZE, false)
            .map_err(VirtioDeviceError::ResourceAlloc)?;
        Ok(Self { dma_buf, avail: 0 })
    }

    fn is_empty(&self) -> bool {
        self.avail == 0
    }

    fn drain_into(&mut self, writer: &mut VmWriter<'_, Fallible>) -> Result<usize, Error> {
        let to_copy = writer.avail().min(self.avail);
        let start = self.avail - to_copy;
        let copied = self
            .dma_buf
            .reader()
            .unwrap()
            .skip(start)
            .limit(to_copy)
            .read_fallible(writer)
            .map_err(|(err, _)| err)?;
        self.avail = start;
        Ok(copied)
    }

    fn swap_in(&mut self, other: &mut DmaStream<FromDevice>, len: usize) {
        debug_assert!(self.is_empty());
        other.sync_from_device(0..len).unwrap();
        core::mem::swap(&mut self.dma_buf, other);
        self.avail = len;
    }
}
