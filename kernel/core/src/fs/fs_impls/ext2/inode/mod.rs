#![short_vis_path::add(ext2)]

mod attrs;
mod block_manager;
mod dir;
mod file;
mod io_range;
mod symlink;
mod sync;

use ostd::const_assert;

use self::{
    block_manager::{BlockPtrTree, InodeBlockManager, RawBlockPtrs},
    symlink::FastSymlinkTarget,
};
use super::{fs::Ext2, prelude::*, xattr::Xattr};
use crate::{
    fs::{ext2::utils, file::InodeMode, pipe::Pipe, vfs::inode::Extension},
    vm::page_cache::Vmo,
};

const MAX_LINK_COUNT: u16 = 32000;
const MAX_FAST_SYMLINK_LEN: usize = size_of::<u32>() * RAW_BLOCK_PTRS_LEN;
pub(super) const RAW_BLOCK_PTRS_LEN: usize = 15;
pub(super) type Iblock = u32;
pub(super) type Ext2Bid = u32;
pub(super) type Ext2Ino = u32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FilePerm(u16);

impl FilePerm {
    pub(super) fn from_bits_truncate(bits: u16) -> Self {
        Self(bits)
    }

    pub(super) fn bits(self) -> u16 {
        self.0
    }
}

pub(crate) struct Inode {
    ino: Ext2Ino,
    type_: InodeType,
    inner: RwMutex<InodeInner>,
    block_group_idx: usize,
    fs: Weak<Ext2>,
    xattr: Option<Xattr>,
    pipe: Option<Pipe>,
    extension: Extension,
}

impl Inode {
    pub(super) fn new(
        ino: Ext2Ino,
        type_: InodeType,
        inode_desc: Dirty<InodeDesc>,
        block_group_idx: usize,
        fs: Weak<Ext2>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self: &Weak<Self>| {
            let xattr = match type_ {
                InodeType::Dir | InodeType::File => Some(Xattr::new(
                    inode_desc.file_acl,
                    weak_self.clone(),
                    fs.clone(),
                )),
                _ => None,
            };
            let pipe = match type_ {
                InodeType::NamedPipe => Some(Pipe::new()),
                _ => None,
            };
            Self {
                ino,
                type_,
                inner: RwMutex::new(InodeInner::new(inode_desc, fs.clone())),
                block_group_idx,
                fs,
                xattr,
                pipe,
                extension: Extension::new(),
            }
        })
    }

    pub(super) fn ino(&self) -> Ext2Ino {
        self.ino
    }

    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    pub(super) fn link_count(&self) -> u16 {
        self.inner.read().link_count()
    }

    pub(super) fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| errno!(EIO, "filesystem already dropped"))
    }

    pub(super) fn file_size(&self) -> usize {
        self.inner.read().file_size()
    }

    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }

    pub(in ext2) fn pipe(&self) -> Option<&Pipe> {
        self.pipe.as_ref()
    }

    pub(super) fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.inner.read().page_cache_clone()
    }
}

impl Drop for Inode {
    fn drop(&mut self) {
        if let Err(err) = self.try_reclaim_deleted_inode() {
            debug!(
                "failed to reclaim deleted inode {} during drop: {:?}",
                self.ino, err
            );
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct InodeDesc {
    type_: InodeType,
    perm: FilePerm,
    uid: u32,
    gid: u32,
    size: u64,
    atime: Duration,
    ctime: Duration,
    mtime: Duration,
    dtime: Duration,
    link_count: u16,
    sector_count: u32,
    flags: FileFlags,
    file_acl: u32,
    generation: u32,
    block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl InodeDesc {
    pub(super) fn new(
        type_: InodeType,
        perm: FilePerm,
        uid: u32,
        gid: u32,
        link_count: u16,
        generation: u32,
        now: Duration,
    ) -> Self {
        Self {
            type_,
            perm,
            uid,
            gid,
            size: 0,
            atime: now,
            ctime: now,
            mtime: now,
            dtime: Duration::ZERO,
            link_count,
            sector_count: 0,
            flags: FileFlags::empty(),
            file_acl: 0,
            generation,
            block_ptrs: [0; RAW_BLOCK_PTRS_LEN],
        }
    }

    pub(super) fn type_(&self) -> InodeType {
        self.type_
    }
}

impl TryFrom<&RawInode> for InodeDesc {
    type Error = Error;
    fn try_from(raw: &RawInode) -> Result<Self> {
        if raw.link_count == 0 {
            return Err(errno!(ESTALE, "inode has been deleted"));
        }

        let mode = raw.mode;
        let type_ = InodeType::from_raw_mode(mode)?;
        let perm = FilePerm::from_bits_truncate(mode & 0o7777);
        let uid = (raw.uid as u32) | ((raw.uid_high as u32) << 16);
        let gid = (raw.gid as u32) | ((raw.gid_high as u32) << 16);
        let atime = Duration::from_secs(raw.atime as u64);
        let ctime = Duration::from_secs(raw.ctime as u64);
        let mtime = Duration::from_secs(raw.mtime as u64);

        let mut size = raw.size_lo as u64;
        if type_ == InodeType::File {
            size |= (raw.size_high as u64) << 32;
        }

        if type_ == InodeType::SymLink && size >= BLOCK_SIZE as u64 {
            return Err(errno!(EUCLEAN, "corrupted symlink on disk"));
        }
        if size > i64::MAX as u64 {
            return Err(errno!(EUCLEAN, "corrupted inode on disk"));
        }

        let flags =
            FileFlags::from_bits(raw.flags).ok_or_else(|| errno!(EIO, "invalid inode flags"))?;
        let raw_block_ptrs = RawBlockPtrs::new(raw.sector_count, raw.block);

        Ok(InodeDesc {
            type_,
            perm,
            uid,
            gid,
            size,
            atime,
            ctime,
            mtime,
            dtime: Duration::from_secs(raw.dtime as u64),
            link_count: raw.link_count,
            sector_count: raw_block_ptrs.sector_count,
            flags,
            file_acl: raw.file_acl,
            generation: raw.generation,
            block_ptrs: raw_block_ptrs.block_ptrs,
        })
    }
}

impl From<&InodeDesc> for RawInode {
    fn from(desc: &InodeDesc) -> Self {
        let mode = (desc.type_ as u16) | (desc.perm.0 & 0o7777);
        let uid = desc.uid as u16;
        let gid = desc.gid as u16;
        let uid_high = (desc.uid >> 16) as u16;
        let gid_high = (desc.gid >> 16) as u16;

        let (size_lo, size_high) = if desc.type_ == InodeType::File {
            (desc.size as u32, (desc.size >> 32) as u32)
        } else {
            (desc.size as u32, 0)
        };

        Self {
            mode,
            uid,
            size_lo,
            atime: utils::duration_to_ext2_secs(desc.atime),
            ctime: utils::duration_to_ext2_secs(desc.ctime),
            mtime: utils::duration_to_ext2_secs(desc.mtime),
            dtime: utils::duration_to_ext2_secs(desc.dtime),
            gid,
            link_count: desc.link_count,
            sector_count: desc.sector_count,
            flags: desc.flags.bits(),
            osd1: 0,
            block: desc.block_ptrs,
            generation: desc.generation,
            file_acl: desc.file_acl,
            size_high,
            faddr: 0,
            frag: 0,
            fsize: 0,
            pad1: 0,
            uid_high,
            gid_high,
            reserved2: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct RawInode {
    pub mode: u16,
    pub uid: u16,
    pub size_lo: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub link_count: u16,
    pub sector_count: u32,
    pub flags: u32,
    pub osd1: u32,
    pub block: [u32; RAW_BLOCK_PTRS_LEN],
    pub generation: u32,
    pub file_acl: u32,
    pub size_high: u32,
    pub faddr: u32,
    pub frag: u8,
    pub fsize: u8,
    pub pad1: u16,
    pub uid_high: u16,
    pub gid_high: u16,
    pub reserved2: u32,
}

const_assert!(size_of::<RawInode>() == 128);

#[derive(Debug)]
struct InodeInner {
    desc: Dirty<InodeDesc>,
    payload: InodePayload,
}

#[derive(Debug)]
enum InodePayload {
    DataBacked {
        page_cache: PageCache,
        block_manager: Arc<InodeBlockManager>,
    },
    FastSymlink {
        target: FastSymlinkTarget,
    },
    Device {
        device_id: u64,
    },
    NoPayload,
}

impl InodeInner {
    fn new(inode_desc: Dirty<InodeDesc>, fs: Weak<Ext2>) -> Self {
        let payload = InodePayload::new(&inode_desc, fs);

        Self {
            desc: inode_desc,
            payload,
        }
    }

    fn page_cache(&self) -> &PageCache {
        self.payload
            .page_cache()
            .expect("data-backed inode must have a page cache")
    }

    fn block_manager(&self) -> Result<&Arc<InodeBlockManager>> {
        self.payload.block_manager()
    }

    fn raw_block_ptrs(&self) -> RawBlockPtrs {
        self.payload.raw_block_ptrs(self.desc.file_acl)
    }

    fn page_cache_clone(&self) -> Option<Arc<Vmo>> {
        self.payload.page_cache().map(PageCache::as_vmo).cloned()
    }

    fn resize_page_cache(&mut self, new_size_bytes: usize, old_size_bytes: usize) -> Result<()> {
        let InodePayload::DataBacked {
            page_cache,
            block_manager,
        } = &mut self.payload
        else {
            return Err(errno!(EINVAL, "inode has no data page cache"));
        };
        page_cache.resize(new_size_bytes, old_size_bytes)?;
        block_manager.set_npages(new_size_bytes.div_ceil(PAGE_SIZE));
        Ok(())
    }

    fn clear_dirty(&mut self) {
        self.desc.clear_dirty();
        if let Ok(block_manager) = self.block_manager() {
            block_manager.clear_dirty();
        }
    }

    fn is_dirty(&self) -> bool {
        self.desc.is_dirty()
            || self
                .block_manager()
                .is_ok_and(|block_manager| block_manager.is_dirty())
    }

    fn inode_type(&self) -> InodeType {
        self.desc.type_
    }

    fn mode(&self) -> InodeMode {
        InodeMode::from_bits_truncate(self.desc.perm.bits())
    }

    fn set_mode(&mut self, mode: InodeMode) {
        self.desc.perm = FilePerm::from_bits_truncate(mode.bits());
    }

    fn uid(&self) -> u32 {
        self.desc.uid
    }

    fn set_uid(&mut self, uid: u32) {
        self.desc.uid = uid;
    }

    fn gid(&self) -> u32 {
        self.desc.gid
    }

    fn set_gid(&mut self, gid: u32) {
        self.desc.gid = gid;
    }

    fn file_size(&self) -> usize {
        self.desc.size as usize
    }

    fn set_file_size(&mut self, new_size: usize) {
        self.desc.size = new_size as u64;
    }

    fn atime(&self) -> Duration {
        self.desc.atime
    }

    fn set_atime(&mut self, time: Duration) {
        self.desc.atime = time;
    }

    fn mtime(&self) -> Duration {
        self.desc.mtime
    }

    fn set_mtime(&mut self, time: Duration) {
        self.desc.mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.desc.ctime
    }

    fn set_ctime(&mut self, time: Duration) {
        self.desc.ctime = time;
    }

    fn set_mtime_ctime(&mut self, time: Duration) {
        self.set_mtime(time);
        self.set_ctime(time);
    }

    fn set_dtime(&mut self, time: Duration) {
        self.desc.dtime = time;
    }

    fn link_count(&self) -> u16 {
        self.desc.link_count
    }

    fn set_link_count(&mut self, count: u16) {
        self.desc.link_count = count;
    }

    fn inc_link_count(&mut self, delta: u16) {
        debug_assert!(self.desc.link_count <= u16::MAX - delta);
        self.desc.link_count += delta;
    }

    fn dec_link_count(&mut self, delta: u16) {
        debug_assert!(self.desc.link_count >= delta);
        self.desc.link_count = self.desc.link_count.saturating_sub(delta);
    }

    fn remove_flags(&mut self, flags: FileFlags) {
        self.desc.flags.remove(flags);
    }

    fn set_file_acl(&mut self, file_acl: u32) {
        self.desc.file_acl = file_acl;
    }
}

impl InodePayload {
    fn new(inode_desc: &Dirty<InodeDesc>, fs: Weak<Ext2>) -> Self {
        let raw_block_ptrs = RawBlockPtrs::new(inode_desc.sector_count, inode_desc.block_ptrs);
        match inode_desc.type_ {
            InodeType::File | InodeType::Dir => {
                Self::new_data_backed(inode_desc.size as usize, raw_block_ptrs, fs)
            }
            InodeType::SymLink if Self::is_fast_symlink(inode_desc) => Self::FastSymlink {
                target: FastSymlinkTarget::new(inode_desc.block_ptrs),
            },
            InodeType::SymLink => {
                Self::new_data_backed(inode_desc.size as usize, raw_block_ptrs, fs)
            }
            InodeType::CharDevice | InodeType::BlockDevice => Self::Device {
                device_id: raw_block_ptrs.read_device_id(),
            },
            _ => Self::NoPayload,
        }
    }

    fn new_data_backed(size: usize, raw_block_ptrs: RawBlockPtrs, fs: Weak<Ext2>) -> Self {
        let page_cache_size = size.align_up(PAGE_SIZE);
        let page_count = page_cache_size / PAGE_SIZE;
        let block_ptr_tree = BlockPtrTree::new(raw_block_ptrs, fs.clone());
        let block_manager = Arc::new(InodeBlockManager::new(
            block_ptr_tree,
            fs.clone(),
            page_count,
        ));
        let page_cache_backend: Weak<dyn PageCacheBackend> = Arc::downgrade(&block_manager) as _;
        let page_cache = PageCache::new_with_backend(page_cache_size, page_cache_backend)
            .expect("ext2 inode page cache allocation failed");

        Self::DataBacked {
            page_cache,
            block_manager,
        }
    }

    fn is_fast_symlink(inode_desc: &Dirty<InodeDesc>) -> bool {
        let xattr_sectors = Self::xattr_sectors(inode_desc.file_acl);
        inode_desc.type_ == InodeType::SymLink && inode_desc.sector_count == xattr_sectors
    }

    fn xattr_sectors(file_acl: u32) -> u32 {
        if file_acl == 0 {
            0
        } else {
            (BLOCK_SIZE / SECTOR_SIZE) as u32
        }
    }

    fn page_cache(&self) -> Option<&PageCache> {
        match self {
            Self::DataBacked { page_cache, .. } => Some(page_cache),
            _ => None,
        }
    }

    fn block_manager(&self) -> Result<&Arc<InodeBlockManager>> {
        match self {
            Self::DataBacked { block_manager, .. } => Ok(block_manager),
            _ => Err(errno!(EINVAL, "inode payload has no block manager",)),
        }
    }

    fn raw_block_ptrs(&self, file_acl: u32) -> RawBlockPtrs {
        match self {
            Self::DataBacked { block_manager, .. } => block_manager.raw_block_ptrs(),
            Self::FastSymlink { target } => {
                RawBlockPtrs::new(Self::xattr_sectors(file_acl), target.block_ptrs())
            }
            Self::Device { device_id } => {
                let mut raw_block_ptrs = RawBlockPtrs::new(0, [0; RAW_BLOCK_PTRS_LEN]);
                raw_block_ptrs.write_device_id(*device_id);
                raw_block_ptrs
            }
            Self::NoPayload => RawBlockPtrs::new(0, [0; RAW_BLOCK_PTRS_LEN]),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct FileFlags: u32 {
        const SECURE_DEL   = 1 << 0;
        const UNDELETE     = 1 << 1;
        const COMPRESS     = 1 << 2;
        const SYNC_UPDATE  = 1 << 3;
        const IMMUTABLE    = 1 << 4;
        const APPEND_ONLY  = 1 << 5;
        const NO_DUMP      = 1 << 6;
        const NO_ATIME     = 1 << 7;
        const DIRTY        = 1 << 8;
        const COMPRESS_BLK = 1 << 9;
        const NO_COMPRESS  = 1 << 10;
        const ENCRYPT      = 1 << 11;
        const INDEX_DIR    = 1 << 12;
        const IMAGIC       = 1 << 13;
        const JOURNAL_DATA = 1 << 14;
        const NO_TAIL      = 1 << 15;
        const DIR_SYNC     = 1 << 16;
        const TOP_DIR      = 1 << 17;
        const RESERVED     = 1 << 31;
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::fs::fs_impls::ext2::test_utils::{self, RawInodeBuilder};

    pub(super) fn read_raw_inode_from_disk(f: &test_utils::Ext2Fixture, ino: u32) -> RawInode {
        let nr_inodes_per_group = f.sb.nr_inodes_per_group();
        let group_idx = ((ino - 1) / nr_inodes_per_group) as usize;
        let inode_idx = (ino - 1) % nr_inodes_per_group;
        let inode_size = f.sb.inode_size();
        let offset_bytes = (inode_idx as usize) * inode_size;
        let block_index = offset_bytes / BLOCK_SIZE;
        let offset_in_block = offset_bytes % BLOCK_SIZE;
        let table_block = f.descs[group_idx].inode_table_bid + block_index as u32;
        f.disk
            .segment()
            .read_val(Bid::new(table_block as u64).to_offset() + offset_in_block)
            .unwrap()
    }

    pub(super) fn make_live_file_inode(
        ext2: &Arc<Ext2>,
        ino: u32,
        size: usize,
        sector_count: u32,
        flags: FileFlags,
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
    ) -> Arc<Inode> {
        let mut raw = RawInodeBuilder::new(0o100644).build();
        raw.size_lo = size as u32;
        raw.sector_count = sector_count;
        raw.flags = flags.bits();
        raw.block = block_ptrs;
        let desc = InodeDesc::try_from(&raw).unwrap();
        Inode::new(
            ino,
            InodeType::File,
            Dirty::new(desc),
            0,
            Arc::downgrade(ext2),
        )
    }
}
