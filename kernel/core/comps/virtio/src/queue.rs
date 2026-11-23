use alloc::{sync::Arc, vec::Vec};
use core::{
    mem::offset_of,
    sync::atomic::{Ordering, fence},
};

use aster_rights::{Dup, TRightSet, TRights, Write};
use aster_util::{field_ptr, safe_ptr::SafePtr};
use bitflags::bitflags;
use ostd::{
    debug,
    mm::{HasPaddr, PodOnce, Split, dma::DmaCoherent},
};

use crate::{
    dma_buf::DmaBuf,
    transport::{
        ConfigManager, VirtioTransport, VirtioTransportError, pci::legacy::VirtioPciLegacyTransport,
    },
};

#[derive(Debug)]
pub(crate) struct VirtQueue {
    descs: Vec<DescriptorSlot>,
    avail: SafePtr<AvailRing, Arc<DmaCoherent>>,
    used: SafePtr<UsedRing, Arc<DmaCoherent>>,
    notify_config: ConfigManager<u32>,

    queue_idx: u32,
    device_queue_size: u16,
    num_used: u16,
    free_head: Option<u16>,
    avail_idx: u16,
    last_used_idx: u16,
    is_callback_enabled: bool,
}

#[derive(Debug)]
pub(crate) enum CreationError {
    InvalidArgs,
    ResourceAlloc(ostd::Error),
    Transport(VirtioTransportError),
}

#[derive(Debug)]
pub(crate) enum AddBufsError {
    InvalidArgs,
    BufferTooSmall,
}

#[derive(Debug)]
pub(crate) enum PopUsedError {
    NotReady,
}

#[derive(Debug)]
struct DescriptorSlot {
    ptr: SafePtr<Descriptor, Arc<DmaCoherent>>,

    next: Option<u16>,
    len: Option<u32>,
}

impl VirtQueue {
    pub(crate) fn new(
        idx: u16,
        size: u16,
        transport: &mut dyn VirtioTransport,
    ) -> Result<Self, CreationError> {
        if !size.is_power_of_two() {
            return Err(CreationError::InvalidArgs);
        }

        let (descriptor_ptr, avail_ring_ptr, used_ring_ptr, device_queue_size) = if transport
            .is_legacy_version()
        {
            let device_queue_size = transport
                .max_queue_size(idx)
                .map_err(CreationError::Transport)? as usize;
            let desc_size = size_of::<Descriptor>() * device_queue_size;

            if !device_queue_size.is_power_of_two()
                || size as usize > device_queue_size
                || device_queue_size > 1024
            {
                return Err(CreationError::InvalidArgs);
            }

            let (dma1, dma2) = {
                let align_size = VirtioPciLegacyTransport::QUEUE_ALIGN_SIZE;
                let total_frames =
                    VirtioPciLegacyTransport::calc_virtqueue_size_aligned(device_queue_size)
                        / align_size;
                let dma =
                    DmaCoherent::alloc(total_frames, true).map_err(CreationError::ResourceAlloc)?;

                let avail_size = size_of::<u16>() * (3 + device_queue_size);
                let seg1_frames = (desc_size + avail_size).div_ceil(align_size);

                dma.split(seg1_frames * align_size)
            };

            let desc_frame_ptr: SafePtr<Descriptor, Arc<DmaCoherent>> =
                SafePtr::new(Arc::new(dma1), 0);
            let mut avail_frame_ptr: SafePtr<AvailRing, Arc<DmaCoherent>> =
                desc_frame_ptr.clone().cast();
            avail_frame_ptr.byte_add(desc_size);
            let used_frame_ptr: SafePtr<UsedRing, Arc<DmaCoherent>> =
                SafePtr::new(Arc::new(dma2), 0);

            (
                desc_frame_ptr,
                avail_frame_ptr,
                used_frame_ptr,
                device_queue_size as u16,
            )
        } else {
            let max_queue_size = transport
                .max_queue_size(idx)
                .map_err(CreationError::Transport)? as usize;

            if size as usize > max_queue_size || size > 256 {
                return Err(CreationError::InvalidArgs);
            }

            let desc_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;
            let avail_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;
            let used_frame = DmaCoherent::alloc(1, true).map_err(CreationError::ResourceAlloc)?;

            (
                SafePtr::new(Arc::new(desc_frame), 0),
                SafePtr::new(Arc::new(avail_frame), 0),
                SafePtr::new(Arc::new(used_frame), 0),
                size,
            )
        };

        debug!("queue_desc start paddr: {:x?}", descriptor_ptr.paddr());
        debug!("queue_driver start paddr: {:x?}", avail_ring_ptr.paddr());
        debug!("queue_device start paddr: {:x?}", used_ring_ptr.paddr());

        transport
            .set_queue(
                idx,
                device_queue_size,
                &descriptor_ptr,
                &avail_ring_ptr,
                &used_ring_ptr,
            )
            .map_err(CreationError::Transport)?;

        let mut descs = Vec::with_capacity(size as usize);
        descs.push(DescriptorSlot {
            ptr: descriptor_ptr,
            next: None,
            len: None,
        });
        for i in 0..size - 1 {
            let last_desc = &mut descs[i as usize];
            let new_desc = {
                let mut ptr = last_desc.ptr.clone();
                ptr.add(1);
                DescriptorSlot {
                    ptr,
                    next: None,
                    len: None,
                }
            };
            last_desc.next = Some(i + 1);
            descs.push(new_desc);
        }

        let notify_config = transport.notify_config(idx as usize);

        field_ptr!(&avail_ring_ptr, AvailRing, flags)
            .write_once(&AvailFlags::empty())
            .unwrap();

        Ok(VirtQueue {
            descs,
            avail: avail_ring_ptr,
            used: used_ring_ptr,
            notify_config,
            queue_idx: idx as u32,
            device_queue_size,
            num_used: 0,
            free_head: Some(0),
            avail_idx: 0,
            last_used_idx: 0,
            is_callback_enabled: true,
        })
    }

    pub(crate) fn add_input_bufs<I: DmaBuf>(&mut self, inputs: &[&I]) -> Result<u16, AddBufsError> {
        self.add_dma_bufs(inputs, &[] as &[&I])
    }

    pub(crate) fn add_output_bufs<O: DmaBuf>(
        &mut self,
        outputs: &[&O],
    ) -> Result<u16, AddBufsError> {
        self.add_dma_bufs(&[] as &[&O], outputs)
    }

    pub(crate) fn add_dma_bufs<I: DmaBuf + ?Sized, O: DmaBuf + ?Sized>(
        &mut self,
        inputs: &[&I],
        outputs: &[&O],
    ) -> Result<u16, AddBufsError> {
        if inputs.is_empty() && outputs.is_empty() {
            return Err(AddBufsError::InvalidArgs);
        }
        if inputs.len() + outputs.len() > self.available_desc() {
            return Err(AddBufsError::BufferTooSmall);
        }

        let head = self.free_head.unwrap();
        let mut output_len = 0;

        let mut last = self.free_head;
        let mut current = self.free_head;
        for input in inputs.iter() {
            let desc = &self.descs[current.unwrap() as usize];
            set_dma_buf(
                &desc.ptr.borrow_vm().restrict::<TRights![Write, Dup]>(),
                *input,
            );
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&DescFlags::NEXT)
                .unwrap();
            if let Some(next) = desc.next {
                field_ptr!(&desc.ptr, Descriptor, next)
                    .write_once(&next)
                    .unwrap();
            }
            last = current;
            current = desc.next;
        }
        for output in outputs.iter() {
            let desc = &self.descs[current.unwrap() as usize];
            output_len += set_dma_buf(
                &desc.ptr.borrow_vm().restrict::<TRights![Write, Dup]>(),
                *output,
            );
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&(DescFlags::NEXT | DescFlags::WRITE))
                .unwrap();
            if let Some(next) = desc.next {
                field_ptr!(&desc.ptr, Descriptor, next)
                    .write_once(&next)
                    .unwrap();
            }
            last = current;
            current = desc.next;
        }
        {
            let desc = &mut self.descs[last.unwrap() as usize];
            self.free_head = desc.next;
            desc.next = None;
            let mut flags: DescFlags = field_ptr!(&desc.ptr, Descriptor, flags)
                .read_once()
                .unwrap();
            flags.remove(DescFlags::NEXT);
            field_ptr!(&desc.ptr, Descriptor, flags)
                .write_once(&flags)
                .unwrap();
        }
        self.num_used += (inputs.len() + outputs.len()) as u16;
        self.descs[head as usize].len = Some(output_len);

        {
            let avail_slot = self.avail_idx & (self.device_queue_size - 1);
            let ring_ptr: SafePtr<[u16; 64], &Arc<DmaCoherent>> =
                field_ptr!(&self.avail, AvailRing, ring);
            let mut ring_slot_ptr = ring_ptr.cast::<u16>();
            ring_slot_ptr.add(avail_slot as usize);
            ring_slot_ptr.write_once(&head).unwrap();
        }
        fence(Ordering::SeqCst);

        self.avail_idx = self.avail_idx.wrapping_add(1);
        field_ptr!(&self.avail, AvailRing, idx)
            .write_once(&self.avail_idx)
            .unwrap();
        fence(Ordering::SeqCst);

        Ok(head)
    }

    pub(crate) fn can_pop(&self) -> bool {
        fence(Ordering::SeqCst);

        self.last_used_idx != field_ptr!(&self.used, UsedRing, idx).read_once().unwrap()
    }

    pub(crate) fn available_desc(&self) -> usize {
        self.descs.len() - self.num_used as usize
    }

    pub(crate) fn pop_used(&mut self) -> Result<(u16, u32), PopUsedError> {
        self.pop_used_with_min_bytes(0)
    }

    pub(crate) fn pop_used_with_min_bytes(
        &mut self,
        min_bytes: usize,
    ) -> Result<(u16, u32), PopUsedError> {
        loop {
            if !self.can_pop() {
                return Err(PopUsedError::NotReady);
            }

            let last_used_slot = self.last_used_idx & (self.device_queue_size - 1);
            let element_ptr = {
                let mut ptr = self.used.borrow_vm();
                ptr.byte_add(offset_of!(UsedRing, ring) + last_used_slot as usize * 8);
                ptr.cast::<UsedElem>()
            };
            let index = field_ptr!(&element_ptr, UsedElem, id).read_once().unwrap();
            let len = field_ptr!(&element_ptr, UsedElem, len).read_once().unwrap();
            self.last_used_idx = self.last_used_idx.wrapping_add(1);

            let (desc, dma_len) = if let Some(desc) = self.descs.get_mut(index as usize)
                && let Some(dma_len) = desc.len
            {
                (desc, dma_len)
            } else {
                ostd::error!(
                    "invalid used token: {} (queue size: {})",
                    index,
                    self.descs.len(),
                );
                continue;
            };
            if len > dma_len || (len as usize) < min_bytes {
                ostd::error!(
                    "invalid used length: {} (expected {}..={})",
                    len,
                    min_bytes,
                    dma_len,
                );
                continue;
            }
            desc.len = None;
            self.recycle_descriptors(index as u16);

            return Ok((index as u16, len));
        }
    }

    fn recycle_descriptors(&mut self, mut head: u16) {
        let origin_free_head = self.free_head;
        self.free_head = Some(head);

        loop {
            let desc = &mut self.descs[head as usize];
            field_ptr!(&desc.ptr, Descriptor, addr)
                .write_once(&(0u64))
                .unwrap();
            field_ptr!(&desc.ptr, Descriptor, len)
                .write_once(&(0u32))
                .unwrap();
            self.num_used -= 1;

            if let Some(next) = desc.next {
                head = next;
            } else {
                desc.next = origin_free_head;
                break;
            }
        }
    }

    pub(crate) fn should_notify(&self) -> bool {
        fence(Ordering::SeqCst);

        let flags = field_ptr!(&self.used, UsedRing, flags).read_once().unwrap();
        flags & 0x0001u16 == 0u16
    }

    pub(crate) fn notify(&mut self) {
        if self.notify_config.is_modern() {
            self.notify_config
                .write_once::<u32>(0, self.queue_idx)
                .unwrap();
        } else {
            self.notify_config
                .write_once::<u16>(0, self.queue_idx as u16)
                .unwrap();
        }
    }

    pub(crate) fn disable_callback(&mut self) {
        if !self.is_callback_enabled {
            return;
        }

        let flags_ptr = field_ptr!(&self.avail, AvailRing, flags);
        let mut flags: AvailFlags = flags_ptr.read_once().unwrap();
        debug_assert!(!flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT));
        flags.insert(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);
        flags_ptr.write_once(&flags).unwrap();

        self.is_callback_enabled = false;
    }

    pub(crate) fn enable_callback(&mut self) {
        if self.is_callback_enabled {
            return;
        }

        let flags_ptr = field_ptr!(&self.avail, AvailRing, flags);
        let mut flags: AvailFlags = flags_ptr.read_once().unwrap();
        debug_assert!(flags.contains(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT));
        flags.remove(AvailFlags::VIRTQ_AVAIL_F_NO_INTERRUPT);
        flags_ptr.write_once(&flags).unwrap();

        self.is_callback_enabled = true;
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct Descriptor {
    addr: u64,
    len: u32,
    flags: DescFlags,
    next: u16,
}

type DescriptorPtr<'a> = SafePtr<Descriptor, &'a Arc<DmaCoherent>, TRightSet<TRights![Dup, Write]>>;

fn set_dma_buf<T: DmaBuf + ?Sized>(desc_ptr: &DescriptorPtr, buf: &T) -> u32 {
    let daddr = buf.daddr();
    let len = buf.len();

    debug_assert!(len < (u32::MAX) as usize);
    debug_assert_ne!(len, 0);

    field_ptr!(desc_ptr, Descriptor, addr)
        .write_once(&(daddr as u64))
        .unwrap();
    field_ptr!(desc_ptr, Descriptor, len)
        .write_once(&(len as u32))
        .unwrap();

    len as u32
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct DescFlags(u16);

bitflags! {
    impl DescFlags: u16 {
        const NEXT     = 1;
        const WRITE    = 2;
        const INDIRECT = 4;

        const _        = !0;
    }
}

impl PodOnce for DescFlags {}

#[repr(C, align(2))]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct AvailRing {
    flags: AvailFlags,
    idx: u16,
    ring: [u16; 64],
    used_event: u16,
}

#[padding_struct]
#[repr(C, align(4))]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct UsedRing {
    flags: u16,
    idx: u16,
    ring: [UsedElem; 64],
    avail_event: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct UsedElem {
    id: u32,
    len: u32,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
struct AvailFlags(u16);

bitflags! {
    impl AvailFlags: u16 {
        const VIRTQ_AVAIL_F_NO_INTERRUPT = 1;

        const _                          = !0;
    }
}

impl PodOnce for AvailFlags {}
