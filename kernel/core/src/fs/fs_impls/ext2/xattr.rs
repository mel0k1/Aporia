use core::cmp::Ordering;

use super::{fs::Ext2, inode::Inode, prelude::*};
use crate::fs::vfs::xattr::{XattrName, XattrNamespace, XattrSetFlags};

const XATTR_NBLOCKS: usize = 1;
const XATTR_MAGIC: u32 = 0xEA02_0000;
const XATTR_ALIGN: usize = 4;
const XATTR_ROUND: usize = XATTR_ALIGN - 1;
const XATTR_HEADER_SIZE: usize = size_of::<XattrHeader>();
const XATTR_ENTRY_HEADER_SIZE: usize = size_of::<XattrEntry>();
const XATTR_TERMINATOR_SIZE: usize = size_of::<u32>();
const XATTR_ENTRY_VALUE_GAP: usize = XATTR_ALIGN;

fn xattr_entry_len(name_len: usize) -> usize {
    (name_len + XATTR_ENTRY_HEADER_SIZE + XATTR_ROUND) & !XATTR_ROUND
}

fn xattr_value_size(size: usize) -> usize {
    (size + XATTR_ROUND) & !XATTR_ROUND
}

#[derive(Debug)]
pub(super) struct Xattr {
    cache: RwMutex<XattrCache>,
}

impl Xattr {
    pub(super) fn new(bid: Ext2Bid, inode: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        Self {
            cache: RwMutex::new(XattrCache {
                block_buf: None,
                entries: Vec::new(),
                bid,
                dirty: false,
                inode,
                fs,
            }),
        }
    }

    pub(super) fn bid(&self) -> Ext2Bid {
        self.cache.read().bid()
    }

    pub(super) fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        let value_len = value_reader.remain();
        if value_len > BLOCK_SIZE {
            return Err(errno!(ERANGE, "xattr value is too large"));
        }

        let mut value = vec![0u8; value_len];
        if value_len > 0 {
            value_reader.read_fallible(&mut VmWriter::from(value.as_mut_slice()))?;
        }

        self.cache
            .write()
            .set_entry(target_index, target_name, value, flags)
    }

    pub(super) fn get_xattr(&self, name: XattrName, vm_writer: &mut VmWriter) -> Result<usize> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        self.cache
            .write()
            .get_entry(target_index, &target_name, vm_writer)
    }

    pub(super) fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.cache.write().list_entries(namespace, list_writer)
    }

    pub(super) fn remove_xattr(&self, name: XattrName) -> Result<()> {
        let (target_index, target_name) = Self::parse_target_name(name)?;
        self.cache.write().remove_entry(target_index, &target_name)
    }

    pub(super) fn delete_xattr_block(&self) -> Result<()> {
        self.cache.write().free_and_invalidate()
    }

    pub(super) fn flush(&self) -> Result<()> {
        self.cache.write().flush()
    }

    fn parse_target_name(name: XattrName) -> Result<(XattrNameIndex, Vec<u8>)> {
        let name_index = XattrNameIndex::from(name.namespace());
        let stripped_name = name_index.strip_prefix(name.full_name())?;
        if stripped_name.len() > u8::MAX as usize {
            return Err(errno!(ERANGE, "xattr name is too long"));
        }
        Ok((name_index, stripped_name.as_bytes().to_vec()))
    }
}

#[derive(Debug)]
struct XattrCache {
    block_buf: Option<USegment>,
    entries: Vec<XattrEntryData>,
    bid: Ext2Bid,
    dirty: bool,
    inode: Weak<Inode>,
    fs: Weak<Ext2>,
}

impl XattrCache {
    fn bid(&self) -> Ext2Bid {
        self.bid
    }

    fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| errno!(EIO, "ext2 instance is unavailable"))
    }

    fn inode(&self) -> Result<Arc<Inode>> {
        self.inode
            .upgrade()
            .ok_or_else(|| errno!(EIO, "inode instance is unavailable"))
    }

    fn set_entry(
        &mut self,
        target_index: XattrNameIndex,
        target_name: Vec<u8>,
        value: Vec<u8>,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.load()?;

        let (found_idx, insert_at) =
            Self::find_entry_position(&self.entries, target_index, &target_name);

        if found_idx.is_some() {
            if flags.contains(XattrSetFlags::CREATE_ONLY) {
                return Err(errno!(EEXIST, "the target xattr already exists"));
            }
        } else if flags.contains(XattrSetFlags::REPLACE_ONLY) {
            return Err(errno!(ENODATA, "the target xattr does not exist"));
        }

        if let Some(idx) = found_idx {
            self.entries[idx].value = value;
        } else {
            self.entries.insert(
                insert_at,
                XattrEntryData {
                    name_index: target_index,
                    name: target_name,
                    value,
                },
            );
        }

        if self.bid == 0 {
            self.alloc_block()?;
        }
        self.dirty = true;
        Ok(())
    }

    fn get_entry(
        &mut self,
        target_index: XattrNameIndex,
        target_name: &[u8],
        vm_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.load()?;

        let (found_idx, _) = Self::find_entry_position(&self.entries, target_index, target_name);
        let idx = found_idx.ok_or_else(|| errno!(ENODATA, "the target xattr does not exist"))?;
        let value = self.entries[idx].value.as_slice();

        if vm_writer.avail() == 0 {
            return Ok(value.len());
        }
        if value.len() > vm_writer.avail() {
            return Err(errno!(ERANGE, "the xattr value buffer is too small"));
        }

        vm_writer.write_fallible(&mut VmReader::from(value))?;
        Ok(value.len())
    }

    fn remove_entry(&mut self, target_index: XattrNameIndex, target_name: &[u8]) -> Result<()> {
        self.load()?;

        let (found_idx, _) = Self::find_entry_position(&self.entries, target_index, target_name);
        let Some(found_idx) = found_idx else {
            return Err(errno!(ENODATA, "the target xattr does not exist"));
        };
        self.entries.remove(found_idx);

        if self.entries.is_empty() {
            self.free_and_invalidate()?;
            return Ok(());
        }

        self.dirty = true;
        Ok(())
    }

    fn list_entries(
        &mut self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        self.load()?;

        let buffer_size = list_writer.avail();
        let mut total_list_size = 0usize;
        let mut remaining_size = buffer_size;

        for entry in &self.entries {
            if namespace == XattrNamespace::User
                && entry.name_index.namespace() != XattrNamespace::User
            {
                continue;
            }

            let prefix = entry.name_index.prefix().as_bytes();
            let list_entry_size = prefix.len() + entry.name.len() + 1;
            total_list_size += list_entry_size;

            if buffer_size > 0 {
                if list_entry_size > remaining_size {
                    return Err(errno!(ERANGE, "the xattr list buffer is too small"));
                }
                list_writer.write_fallible(&mut VmReader::from(prefix))?;
                list_writer.write_fallible(&mut VmReader::from(entry.name.as_slice()))?;
                list_writer.write_fallible(&mut VmReader::from(&[0u8][..]))?;
                remaining_size -= list_entry_size;
            }
        }

        if buffer_size > 0 {
            Ok(buffer_size - remaining_size)
        } else {
            Ok(total_list_size)
        }
    }

    fn find_entry_position(
        entries: &[XattrEntryData],
        target_index: XattrNameIndex,
        target_name: &[u8],
    ) -> (Option<usize>, usize) {
        let mut found_idx = None;
        let mut insert_at = entries.len();

        for (idx, entry) in entries.iter().enumerate() {
            match cmp_entry_key(entry.name_index, &entry.name, target_index, target_name) {
                Ordering::Less => continue,
                Ordering::Equal => {
                    found_idx = Some(idx);
                    insert_at = idx;
                    break;
                }
                Ordering::Greater => {
                    insert_at = idx;
                    break;
                }
            }
        }

        (found_idx, insert_at)
    }

    fn load(&mut self) -> Result<()> {
        if self.block_buf.is_some() {
            return Ok(());
        }
        if self.bid == 0 {
            return Ok(());
        }

        let fs = self.fs()?;
        let block_buf = Self::alloc_block_buffer()?;

        let bio_segment = BioSegment::new_from_segment(block_buf.clone(), BioDirection::FromDevice);
        fs.read_blocks(self.bid, bio_segment)?;

        Self::validate_block(&block_buf)?;
        let entries = Self::parse_entries_from_block(&block_buf)?;
        self.entries = entries;
        self.block_buf = Some(block_buf);
        Ok(())
    }

    fn alloc_block(&mut self) -> Result<()> {
        debug_assert!(self.bid == 0);

        let fs = self.fs()?;
        let inode = self.inode()?;
        let goal = {
            let sb = fs.super_block();
            sb.group_first_block_no(inode.block_group_idx())
        };
        let range = fs.alloc_blocks(1, goal)?;
        self.bid = range.start;
        if self.block_buf.is_none() {
            self.block_buf = Some(Self::alloc_block_buffer()?);
        }
        Ok(())
    }

    fn free_and_invalidate(&mut self) -> Result<()> {
        if self.bid != 0 {
            let fs = self.fs()?;
            fs.free_blocks(self.bid, 1)?;
        }
        self.bid = 0;
        self.block_buf = None;
        self.entries = Vec::new();
        self.dirty = false;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        if self.bid == 0 {
            self.dirty = false;
            return Ok(());
        }

        let fs = self.fs()?;
        let block_buf = self.block_buf.as_ref().unwrap();
        self.write_entries_to_segment(block_buf)?;

        let bio_segment = BioSegment::new_from_segment(block_buf.clone(), BioDirection::ToDevice);
        fs.write_blocks(self.bid, bio_segment)?;
        self.dirty = false;
        Ok(())
    }

    fn write_entries_to_segment(&self, block: &USegment) -> Result<()> {
        let entries = &self.entries;

        block
            .fill_zeros(0, BLOCK_SIZE)
            .map_err(|(error, _)| error)?;

        let header = XattrHeader {
            magic: XATTR_MAGIC,
            ref_count: 1,
            nblocks: XATTR_NBLOCKS as u32,
            hash: 0,
            reserved: [0u32; 4],
        };
        block.write_val(0, &header)?;

        let mut entry_cursor = XATTR_HEADER_SIZE;
        let mut value_cursor = BLOCK_SIZE;

        for entry in entries {
            let entry_padded_size = xattr_entry_len(entry.name.len());
            let value_padded_size = xattr_value_size(entry.value.len());

            value_cursor = value_cursor
                .checked_sub(value_padded_size)
                .ok_or_else(|| errno!(ENOSPC, "insufficient xattr value space"))?;

            if entry_cursor + entry_padded_size + XATTR_ENTRY_VALUE_GAP > value_cursor {
                return Err(errno!(ENOSPC, "xattr entry/value regions overlap"));
            }

            if !entry.value.is_empty() {
                block.write_bytes(value_cursor, &entry.value)?;
            }

            let raw_entry = XattrEntry {
                name_len: entry.name.len() as u8,
                name_index: entry.name_index as u8,
                value_offset: if entry.value.is_empty() {
                    0
                } else {
                    value_cursor as u16
                },
                value_block: 0,
                value_len: entry.value.len() as u32,
                hash: 0,
            };
            block.write_val(entry_cursor, &raw_entry)?;

            let name_offset = entry_cursor + XATTR_ENTRY_HEADER_SIZE;
            block.write_bytes(name_offset, &entry.name)?;

            entry_cursor += entry_padded_size;
        }

        let terminator = XattrEntry {
            name_len: 0,
            name_index: 0,
            value_offset: 0,
            value_block: 0,
            value_len: 0,
            hash: 0,
        };
        block.write_val(entry_cursor, &terminator)?;

        Ok(())
    }

    fn alloc_block_buffer() -> Result<USegment> {
        let segment = FrameAllocOptions::new().zeroed(true).alloc_segment(1)?;
        Ok(segment.into())
    }

    fn validate_header(block_buf: &USegment) -> Result<()> {
        let header = block_buf.read_val::<XattrHeader>(0)?;
        if header.magic != XATTR_MAGIC || header.nblocks != XATTR_NBLOCKS as u32 {
            return Err(errno!(EIO, "invalid xattr header"));
        }
        Ok(())
    }

    fn validate_entry(entry: &XattrEntry, offset: usize) -> Result<()> {
        let entry_len = xattr_entry_len(entry.name_len as usize);
        let next_entry_offset = offset + entry_len;
        if next_entry_offset >= BLOCK_SIZE {
            return Err(errno!(EIO, "xattr entry overflows block"));
        }
        if entry.value_block != 0 {
            return Err(errno!(EIO, "xattr external value blocks are not supported"));
        }

        let value_size = entry.value_len as usize;
        let value_offset = entry.value_offset as usize;
        if value_size > BLOCK_SIZE {
            return Err(errno!(EIO, "xattr value range is out of block bounds"));
        }
        let value_end = value_offset + value_size;
        if value_end > BLOCK_SIZE {
            return Err(errno!(EIO, "xattr value range is out of block bounds"));
        }
        Ok(())
    }

    fn validate_block(block_buf: &USegment) -> Result<()> {
        Self::validate_header(block_buf)?;

        let mut offset = XATTR_HEADER_SIZE;
        loop {
            if offset + XATTR_TERMINATOR_SIZE > BLOCK_SIZE {
                return Err(errno!(EIO, "xattr entry terminator is missing"));
            }

            let marker = block_buf.read_val::<u32>(offset)?;
            if marker == 0 {
                return Ok(());
            }

            let entry = block_buf.read_val::<XattrEntry>(offset)?;
            let _ = XattrNameIndex::try_from(entry.name_index)
                .map_err(|_| errno!(EIO, "invalid xattr name index on disk"))?;
            Self::validate_entry(&entry, offset)?;
            offset += xattr_entry_len(entry.name_len as usize);
        }
    }

    fn parse_entries_from_block(block_buf: &USegment) -> Result<Vec<XattrEntryData>> {
        let mut entries: Vec<XattrEntryData> = Vec::new();
        let mut offset = XATTR_HEADER_SIZE;
        loop {
            if offset + XATTR_TERMINATOR_SIZE > BLOCK_SIZE {
                return Err(errno!(EIO, "xattr entry terminator is missing"));
            }

            let marker = block_buf.read_val::<u32>(offset)?;
            if marker == 0 {
                break;
            }

            let entry = block_buf.read_val::<XattrEntry>(offset)?;
            let name_index = XattrNameIndex::try_from(entry.name_index)
                .map_err(|_| errno!(EIO, "invalid xattr name index on disk"))?;
            Self::validate_entry(&entry, offset)?;

            let name_len = entry.name_len as usize;
            let mut name = vec![0u8; name_len];
            block_buf.read_bytes(offset + XATTR_ENTRY_HEADER_SIZE, &mut name)?;

            if let Some(prev_entry) = entries.last() {
                let ordering =
                    cmp_entry_key(prev_entry.name_index, &prev_entry.name, name_index, &name);
                if ordering != Ordering::Less {
                    return Err(errno!(EIO, "xattr entries are not strictly sorted"));
                }
            }

            let value_len = entry.value_len as usize;
            let mut value = vec![0u8; value_len];
            if value_len > 0 {
                block_buf.read_bytes(entry.value_offset as usize, &mut value)?;
            }

            entries.push(XattrEntryData {
                name_index,
                name,
                value,
            });

            offset += xattr_entry_len(name_len);
        }

        Ok(entries)
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum XattrNameIndex {
    User = 1,
    PosixAclAccess = 2,
    PosixAclDefault = 3,
    Trusted = 4,
    Lustre = 5,
    Security = 6,
}

impl From<XattrNamespace> for XattrNameIndex {
    fn from(namespace: XattrNamespace) -> Self {
        match namespace {
            XattrNamespace::User => Self::User,
            XattrNamespace::Trusted => Self::Trusted,
            XattrNamespace::Security => Self::Security,
            XattrNamespace::System => Self::PosixAclAccess,
        }
    }
}

impl XattrNameIndex {
    pub(super) fn prefix(self) -> &'static str {
        match self {
            Self::User => "user.",
            Self::PosixAclAccess => "system.posix_acl_access.",
            Self::PosixAclDefault => "system.posix_acl_default.",
            Self::Trusted => "trusted.",
            Self::Lustre => "lustre.",
            Self::Security => "security.",
        }
    }

    pub(super) fn strip_prefix(self, full_name: &str) -> Result<&str> {
        full_name.strip_prefix(self.prefix()).ok_or_else(|| {
            errno!(
                EINVAL,
                "xattr name does not match namespace-specific prefix",
            )
        })
    }

    fn namespace(self) -> XattrNamespace {
        match self {
            Self::User => XattrNamespace::User,
            Self::Trusted => XattrNamespace::Trusted,
            Self::Security => XattrNamespace::Security,
            Self::PosixAclAccess | Self::PosixAclDefault | Self::Lustre => XattrNamespace::System,
        }
    }
}

#[derive(Clone, Debug)]
struct XattrEntryData {
    name_index: XattrNameIndex,
    name: Vec<u8>,
    value: Vec<u8>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct XattrHeader {
    magic: u32,
    ref_count: u32,
    nblocks: u32,
    hash: u32,
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct XattrEntry {
    name_len: u8,
    name_index: u8,
    value_offset: u16,
    value_block: u32,
    value_len: u32,
    hash: u32,
}

fn cmp_entry_key(
    lhs_index: XattrNameIndex,
    lhs_name: &[u8],
    rhs_index: XattrNameIndex,
    rhs_name: &[u8],
) -> Ordering {
    (lhs_index as u8)
        .cmp(&(rhs_index as u8))
        .then(lhs_name.len().cmp(&rhs_name.len()))
        .then(lhs_name.cmp(rhs_name))
}
