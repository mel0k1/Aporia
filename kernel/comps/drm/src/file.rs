use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use aster_core::{
    current,
    events::IoEvents,
    fs::{
        file::{Mappable, MappableObject, MappedObject, PerOpenFileOps, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::{
        Process,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
    vm::vmar::{FileMmapRequest, MapHandle},
};
use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use hashbrown::HashMap;
use ostd::{
    mm::{PAGE_SIZE, VmReader, VmWriter},
    sync::Mutex,
};
use sparse_id_alloc::SparseIdAlloc;

use crate::{
    device::{DrmDevice, DrmMaster},
    gem::object::DrmGemObject,
    has_current_sys_admin,
    kms::objects::{KmsObjectId, framebuffer::DrmFramebuffer},
    minor::{DrmMinor, DrmMinorType},
};

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(super) struct DrmFile {
    client_id: u64,
    client_caps: AtomicDrmClientCaps,
    auth: Option<DrmPrimaryAuth>,
    minor: Arc<DrmMinor>,

    gem_handles: Mutex<DrmGemHandleTable>,
    framebuffers: Mutex<Vec<Arc<DrmFramebuffer>>>,
}

impl DrmFile {
    pub(super) fn device(&self) -> &Arc<dyn DrmDevice> {
        self.minor.device()
    }

    pub(super) fn minor_type(&self) -> DrmMinorType {
        self.minor.type_()
    }

    pub(super) fn has_client_caps(&self, cap: DrmClientCaps) -> bool {
        self.client_caps.load(Ordering::Relaxed).contains(cap)
    }

    pub(super) fn new(minor: Arc<DrmMinor>) -> Self {
        let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
        let auth = minor.open_client(client_id).map(|master| {
            let is_master = master.owner_client_id() == client_id;

            DrmPrimaryAuth {
                state: Mutex::new(DrmPrimaryAuthState {
                    owner_process: Arc::downgrade(&current!()),
                    was_master: is_master,
                    magic: None,
                    master,
                }),
                authenticated: Arc::new(AtomicBool::new(is_master)),
            }
        });

        Self {
            client_id,
            client_caps: AtomicDrmClientCaps::default(),
            auth,
            minor,

            gem_handles: Mutex::new(DrmGemHandleTable::new()),
            framebuffers: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn is_master(&self) -> bool {
        self.minor.is_client_master(self.client_id)
    }

    pub(super) fn set_client_caps(&self, caps: DrmClientCaps, enabled: bool) {
        self.client_caps.set(caps, enabled);
    }

    pub(super) fn is_authenticated(&self) -> bool {
        self.auth
            .as_ref()
            .is_some_and(|auth| auth.authenticated.load(Ordering::Relaxed))
    }

    pub(super) fn get_or_allocate_magic(&self) -> Result<u32> {
        let auth = self.primary_auth()?;
        let mut auth_state = auth.state.lock();
        if let Some(magic) = auth_state.magic {
            return Ok(magic);
        }

        let magic = auth_state.master.allocate_magic(&auth.authenticated)?;
        auth_state.magic = Some(magic);
        Ok(magic)
    }

    pub(super) fn authenticate_magic(&self, magic: u32) -> Result<()> {
        self.minor.authenticate_magic(self.client_id, magic)
    }

    pub(super) fn set_master(&self) -> Result<()> {
        self.check_master_control_permission()?;

        let owner_process = current!();
        let auth = self.primary_auth()?;
        let mut auth_state = auth.state.lock();
        let retained_master = auth_state.was_master.then_some(&auth_state.master);
        let master = self.minor.set_master(self.client_id, retained_master)?;

        if !Arc::ptr_eq(&auth_state.master, &master)
            && let Some(magic) = auth_state.magic.take()
        {
            auth_state.master.release_magic(magic);
        }

        if !auth_state.was_master {
            auth_state.owner_process = Arc::downgrade(&owner_process);
        }
        auth_state.was_master = true;
        auth_state.master = master;
        auth.authenticated.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub(super) fn drop_master(&self) -> Result<()> {
        self.check_master_control_permission()?;
        self.minor.drop_master(self.client_id)
    }

    pub(super) fn add_gem_object(&self, gem_object: Arc<DrmGemObject>) -> Result<u32> {
        let handle = self.gem_handles.lock().insert(Arc::clone(&gem_object))?;
        gem_object.allow_mmap(self.client_id);
        Ok(handle)
    }

    pub(super) fn lookup_gem_object(&self, handle: u32) -> Result<Arc<DrmGemObject>> {
        self.gem_handles
            .lock()
            .objects
            .get(&handle)
            .cloned()
            .ok_or_else(|| errno!(ENOENT, "the GEM handle is invalid"))
    }

    pub(super) fn map_gem_handle(&self, handle: u32) -> Result<u64> {
        let gem_object = self.lookup_gem_object(handle)?;
        self.minor.gem_mmap_offset(&gem_object)
    }

    pub(super) fn remove_gem_object(&self, handle: u32) -> Result<()> {
        let mut gem_handles = self.gem_handles.lock();
        let Some(gem_object) = gem_handles.remove(handle) else {
            return Err(errno!(EINVAL, "the GEM handle is invalid"));
        };
        drop(gem_handles);

        gem_object.revoke_mmap(self.client_id);
        Ok(())
    }

    pub(super) fn add_framebuffer(&self, framebuffer: Arc<DrmFramebuffer>) {
        let mut framebuffers = self.framebuffers.lock();
        framebuffers.push(framebuffer);
    }

    pub(super) fn lookup_framebuffer(&self, id: KmsObjectId) -> Option<Arc<DrmFramebuffer>> {
        self.framebuffers
            .lock()
            .iter()
            .find(|framebuffer| framebuffer.id() == id)
            .cloned()
    }

    pub(super) fn framebuffer_ids(&self) -> Vec<KmsObjectId> {
        self.framebuffers
            .lock()
            .iter()
            .map(|framebuffer| framebuffer.id())
            .collect()
    }

    pub(super) fn remove_framebuffer(&self, id: KmsObjectId) -> Option<Arc<DrmFramebuffer>> {
        let mut framebuffers = self.framebuffers.lock();
        let position = framebuffers
            .iter()
            .position(|framebuffer| framebuffer.id() == id)?;
        Some(framebuffers.swap_remove(position))
    }

    fn update_owner_process(&self) {
        let Some(auth) = self.auth.as_ref() else {
            return;
        };
        let mut auth_state = auth.state.lock();

        if auth_state.was_master {
            return;
        }

        auth_state.owner_process = Arc::downgrade(&current!());
    }

    fn check_master_control_permission(&self) -> Result<()> {
        if has_current_sys_admin() {
            return Ok(());
        }

        let auth = self.primary_auth()?;
        let auth_state = auth.state.lock();
        let is_owner_process = auth_state
            .owner_process
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, &current!()));

        if auth_state.was_master && is_owner_process {
            Ok(())
        } else {
            Err(errno!(
                EACCES,
                "the DRM master control requires CAP_SYS_ADMIN or ownership"
            ))
        }
    }

    fn primary_auth(&self) -> Result<&DrmPrimaryAuth> {
        self.auth
            .as_ref()
            .ok_or_else(|| errno!(EACCES, "the DRM operation requires a primary-node file",))
    }
}

impl PerOpenFileOps for DrmFile {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn mappable(&self, request: FileMmapRequest) -> Result<MappableObject<'_>> {
        if !request.is_shared() {
            return Err(errno!(EINVAL, "private DRM mappings are not supported"));
        }

        Ok(MappableObject::Device(self as &dyn Mappable))
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        self.update_owner_process();
        self.dispatch_ioctl(raw_ioctl)
    }
}

impl Mappable for DrmFile {
    fn map(&self, offset: usize, handle: MapHandle) -> Result<Box<dyn MappedObject>> {
        let size = handle.size();

        debug_assert!(offset.is_multiple_of(PAGE_SIZE));
        debug_assert!(size.is_multiple_of(PAGE_SIZE));

        self.minor
            .create_gem_mapped_object(self.client_id, offset, size)
    }
}

impl FileOps for DrmFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EINVAL, "reading from a DRM file is not supported"))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(EINVAL, "writing to a DRM file is not supported"))
    }
}

impl Pollable for DrmFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl Drop for DrmFile {
    fn drop(&mut self) {
        let device = self.minor.device().clone();
        if let Some(kms_ops) = device.as_kms_ops() {
            let mut object_store = kms_ops.mode_config().object_store().lock();
            for framebuffer in self.framebuffers.get_mut().drain(..) {
                object_store.free_object_id(framebuffer.id());
            }
        }

        for gem_object in self.gem_handles.get_mut().drain() {
            gem_object.revoke_mmap(self.client_id);
        }

        let Some(auth) = self.auth.as_ref() else {
            return;
        };

        let mut auth_state = auth.state.lock();
        if let Some(magic) = auth_state.magic.take() {
            auth_state.master.release_magic(magic);
        }
        drop(auth_state);

        let _ = self.minor.drop_master(self.client_id);
    }
}

#[derive(Debug)]
struct DrmPrimaryAuth {
    state: Mutex<DrmPrimaryAuthState>,
    authenticated: Arc<AtomicBool>,
}

#[derive(Debug)]
struct DrmPrimaryAuthState {
    owner_process: Weak<Process>,
    was_master: bool,
    magic: Option<u32>,
    master: Arc<DrmMaster>,
}

#[derive(Debug)]
struct DrmGemHandleTable {
    allocator: SparseIdAlloc,
    objects: HashMap<u32, Arc<DrmGemObject>>,
}

impl DrmGemHandleTable {
    fn new() -> Self {
        Self {
            allocator: SparseIdAlloc::new(1, u32::MAX),
            objects: HashMap::new(),
        }
    }

    fn insert(&mut self, object: Arc<DrmGemObject>) -> Result<u32> {
        let Some(handle) = self.allocator.alloc() else {
            return Err(errno!(ENOMEM, "no GEM handles are available"));
        };

        let previous = self.objects.insert(handle, object);
        debug_assert!(previous.is_none(), "an allocated GEM handle must be unused");
        Ok(handle)
    }

    fn remove(&mut self, handle: u32) -> Option<Arc<DrmGemObject>> {
        let object = self.objects.remove(&handle)?;
        self.allocator.free(handle);
        Some(object)
    }

    fn drain(&mut self) -> impl Iterator<Item = Arc<DrmGemObject>> + '_ {
        self.allocator = SparseIdAlloc::new(1, u32::MAX);
        self.objects.drain().map(|(_, object)| object)
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(super) struct DrmClientCaps: u32 {
        const STEREO_3D            = 1 << 0;
        const UNIVERSAL_PLANES     = 1 << 1;
        const ATOMIC               = 1 << 2;
        const ASPECT_RATIO         = 1 << 3;
        const WRITEBACK_CONNECTORS = 1 << 4;
        const CURSOR_PLANE_HOTSPOT = 1 << 5;
    }
}

impl From<u32> for DrmClientCaps {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<DrmClientCaps> for u32 {
    fn from(value: DrmClientCaps) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(DrmClientCaps, {
    #[derive(Debug, Default)]
    struct AtomicDrmClientCaps(AtomicU32);
});

impl AtomicDrmClientCaps {
    fn set(&self, flags: DrmClientCaps, enabled: bool) {
        self.update(Ordering::Relaxed, Ordering::Relaxed, |mut caps| {
            caps.set(flags, enabled);
            caps
        });
    }
}
