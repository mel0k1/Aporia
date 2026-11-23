use aster_virtio::device::entropy::{self, device::EntropyDevice};
use device_id::{DeviceId, MinorId};

use crate::{
    device::{Device, DeviceType, registry::char},
    events::IoEvents,
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

const HWRNG_MINOR: u32 = 183;

static RNG_CURRENT: Mutex<Option<Arc<EntropyDevice>>> = Mutex::new(None);

#[derive(Debug)]
struct HwRngDevice {
    id: DeviceId,
}

impl HwRngDevice {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(HWRNG_MINOR);

        let id = DeviceId::new(major, minor);
        Arc::new(Self { id })
    }
}

impl Device for HwRngDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        Some(DevtmpfsNodeMeta::new("hwrng").unwrap())
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(HwRngFile))
    }
}

struct HwRngFile;

impl Pollable for HwRngFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        mask & (IoEvents::IN | IoEvents::OUT)
    }
}

impl FileOps for HwRngFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let dev = current_device()?;
        let is_nonblocking = status_flags.contains(StatusFlags::O_NONBLOCK);

        let mut total_copied: usize = 0;
        while writer.avail() > 0 {
            let mut new_writer = writer.clone_exclusive();

            let res = if is_nonblocking {
                match dev.try_read(&mut new_writer)? {
                    Some(copied) => Ok(copied),
                    None => return Err(errno!(EAGAIN, "no entropy is available")),
                }
            } else {
                dev.wait_queue()
                    .wait_until(|| match dev.try_read(&mut new_writer) {
                        Ok(Some(copied)) => Some(Ok(copied)),
                        Ok(None) => None,
                        Err(err) => Some(Err(err)),
                    })
            };

            match res {
                Ok(copied) => {
                    writer.skip(copied);
                    total_copied += copied;
                }
                Err(err) => {
                    if total_copied == 0 {
                        return Err(err.into());
                    }
                    break;
                }
            }
        }

        Ok(total_copied)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(
            EBADF,
            "the hardware RNG device does not support writing"
        ))
    }
}

impl PerOpenFileOps for HwRngFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

fn current_device() -> Result<Arc<EntropyDevice>> {
    let Some(rng) = RNG_CURRENT.lock().clone() else {
        return Err(errno!(ENODEV, "no current hardware RNG device is selected"));
    };
    Ok(rng)
}

pub(super) fn init_in_first_kthread() {
    if let Some(device) = entropy::first_device() {
        *RNG_CURRENT.lock() = Some(device);
    }

    char::register(HwRngDevice::new()).unwrap();
}
