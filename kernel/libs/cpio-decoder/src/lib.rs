#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]
#![feature(alloc_io)]

extern crate alloc;

#[cfg(not(test))]
use alloc::string::{String, ToString};
use alloc::{
    io::{Read, Write},
    vec,
};
use core::cmp::min;

use int_to_c_enum::TryFromInt;
use lending_iterator::prelude::*;

use crate::error::{Error, Result};

pub mod error;

#[cfg(test)]
mod test;

pub struct CpioDecoder<R> {
    reader: R,
    is_error: bool,
}

impl<R> CpioDecoder<R>
where
    R: Read,
{
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            is_error: false,
        }
    }
}

#[gat]
impl<R> LendingIterator for CpioDecoder<R>
where
    R: Read,
{
    type Item<'a> = Result<CpioEntry<'a, R>>;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        if self.is_error {
            return None;
        }

        let entry_result = CpioEntry::new(&mut self.reader);
        match &entry_result {
            Ok(entry) => {
                if entry.is_trailer() {
                    return None;
                }
            }
            Err(_) => {
                self.is_error = true;
            }
        }
        Some(entry_result)
    }
}

#[derive(Debug)]
pub struct CpioEntry<'a, R: Read> {
    metadata: FileMetadata,
    name: String,
    reader: &'a mut R,
    unread_data: usize,
}

impl<'a, R> CpioEntry<'a, R>
where
    R: Read,
{
    fn new(reader: &'a mut R) -> Result<Self> {
        let (metadata, name) = {
            let header = Header::new(reader)?;
            let name = {
                let name_size = read_hex_bytes_to_u32(&header.name_size)? as usize;
                let mut name_bytes = vec![0u8; name_size];
                reader.read_exact(&mut name_bytes)?;
                let name = core::ffi::CStr::from_bytes_with_nul(&name_bytes)
                    .map_err(|_| Error::FileNameError)?;
                name.to_str().map_err(|_| Error::Utf8Error)?.to_string()
            };
            let metadata = if name == TRAILER_NAME {
                Default::default()
            } else {
                FileMetadata::new(&header)?
            };
            let header_padding_len = align_up_pad(header.len() + name.len() + 1, 4);
            if header_padding_len > 0 {
                let mut pad_buf = vec![0u8; header_padding_len];
                reader.read_exact(&mut pad_buf)?;
            }

            (metadata, name)
        };
        let unread_data = metadata.size() as usize + align_up_pad(metadata.size() as usize, 4);
        Ok(Self {
            metadata,
            name,
            reader,
            unread_data,
        })
    }

    pub fn metadata(&self) -> &FileMetadata {
        &self.metadata
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn read_all<W>(&mut self, mut writer: W) -> Result<()>
    where
        W: Write,
    {
        let data_len = self.metadata().size() as usize;
        let data_padding_len = align_up_pad(data_len, 4);
        let mut send_len = 0;
        let mut buffer = vec![0u8; 0x1000];
        while send_len < data_len {
            let len = min(buffer.len(), data_len - send_len);
            self.reader.read_exact(&mut buffer[..len])?;
            self.unread_data -= len;
            writer.write_all(&buffer[..len])?;
            send_len += len;
        }
        if data_padding_len > 0 {
            self.reader.read_exact(&mut buffer[..data_padding_len])?;
            self.unread_data -= data_padding_len;
        }
        Ok(())
    }

    pub fn is_trailer(&self) -> bool {
        self.name == TRAILER_NAME
    }
}

impl<R: Read> Drop for CpioEntry<'_, R> {
    fn drop(&mut self) {
        let mut buffer = [0u8; 512];
        while self.unread_data > 0 {
            let chunk = min(buffer.len(), self.unread_data);
            if self.reader.read_exact(&mut buffer[..chunk]).is_err() {
                break;
            }
            self.unread_data -= chunk;
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileMetadata {
    ino: u32,
    type_: FileType,
    mode: u16,
    uid: u32,
    gid: u32,
    nlink: u32,
    mtime: u32,
    size: u32,
    dev_maj: u32,
    dev_min: u32,
    rdev_maj: u32,
    rdev_min: u32,
}

impl FileMetadata {
    fn new(header: &Header) -> Result<Self> {
        const MODE_MASK: u32 = 0o7777;
        const TYPE_MASK: u32 = 0o170000;
        let raw_mode = read_hex_bytes_to_u32(&header.mode)?;
        let metadata = Self {
            ino: read_hex_bytes_to_u32(&header.ino)?,
            type_: FileType::try_from(raw_mode & TYPE_MASK).map_err(|_| Error::FileTypeError)?,
            mode: (raw_mode & MODE_MASK) as u16,
            uid: read_hex_bytes_to_u32(&header.uid)?,
            gid: read_hex_bytes_to_u32(&header.gid)?,
            nlink: read_hex_bytes_to_u32(&header.nlink)?,
            mtime: read_hex_bytes_to_u32(&header.mtime)?,
            size: read_hex_bytes_to_u32(&header.file_size)?,
            dev_maj: read_hex_bytes_to_u32(&header.dev_maj)?,
            dev_min: read_hex_bytes_to_u32(&header.dev_min)?,
            rdev_maj: read_hex_bytes_to_u32(&header.rdev_maj)?,
            rdev_min: read_hex_bytes_to_u32(&header.rdev_min)?,
        };
        Ok(metadata)
    }

    pub fn ino(&self) -> u32 {
        self.ino
    }

    pub fn file_type(&self) -> FileType {
        self.type_
    }

    pub fn permission_mode(&self) -> u16 {
        self.mode
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn gid(&self) -> u32 {
        self.gid
    }

    pub fn nlink(&self) -> u32 {
        self.nlink
    }

    pub fn mtime(&self) -> u32 {
        self.mtime
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn dev_maj(&self) -> u32 {
        self.dev_maj
    }

    pub fn dev_min(&self) -> u32 {
        self.dev_min
    }

    pub fn rdev_maj(&self) -> u32 {
        self.rdev_maj
    }

    pub fn rdev_min(&self) -> u32 {
        self.rdev_min
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, TryFromInt)]
pub enum FileType {
    FiFo = 0o010000,
    Char = 0o020000,
    Dir = 0o040000,
    Block = 0o060000,
    #[default]
    File = 0o100000,
    Link = 0o120000,
    Socket = 0o140000,
}

const MAGIC: &[u8] = b"070701";
const TRAILER_NAME: &str = "TRAILER!!!";

struct Header {
    magic: [u8; 6],
    ino: [u8; 8],
    mode: [u8; 8],
    uid: [u8; 8],
    gid: [u8; 8],
    nlink: [u8; 8],
    mtime: [u8; 8],
    file_size: [u8; 8],
    dev_maj: [u8; 8],
    dev_min: [u8; 8],
    rdev_maj: [u8; 8],
    rdev_min: [u8; 8],
    name_size: [u8; 8],
    _chksum: [u8; 8],
}

impl Header {
    fn new<R>(reader: &mut R) -> Result<Self>
    where
        R: Read,
    {
        let mut buf = vec![0u8; size_of::<Self>()];
        reader.read_exact(&mut buf)?;

        let header = Self {
            magic: <[u8; 6]>::try_from(&buf[0..6]).unwrap(),
            ino: <[u8; 8]>::try_from(&buf[6..14]).unwrap(),
            mode: <[u8; 8]>::try_from(&buf[14..22]).unwrap(),
            uid: <[u8; 8]>::try_from(&buf[22..30]).unwrap(),
            gid: <[u8; 8]>::try_from(&buf[30..38]).unwrap(),
            nlink: <[u8; 8]>::try_from(&buf[38..46]).unwrap(),
            mtime: <[u8; 8]>::try_from(&buf[46..54]).unwrap(),
            file_size: <[u8; 8]>::try_from(&buf[54..62]).unwrap(),
            dev_maj: <[u8; 8]>::try_from(&buf[62..70]).unwrap(),
            dev_min: <[u8; 8]>::try_from(&buf[70..78]).unwrap(),
            rdev_maj: <[u8; 8]>::try_from(&buf[78..86]).unwrap(),
            rdev_min: <[u8; 8]>::try_from(&buf[86..94]).unwrap(),
            name_size: <[u8; 8]>::try_from(&buf[94..102]).unwrap(),
            _chksum: <[u8; 8]>::try_from(&buf[102..110]).unwrap(),
        };
        if header.magic != MAGIC {
            return Err(Error::MagicError);
        }
        Ok(header)
    }

    fn len(&self) -> usize {
        size_of::<Self>()
    }
}

fn read_hex_bytes_to_u32(bytes: &[u8]) -> Result<u32> {
    debug_assert!(bytes.len() == 8);
    let string = core::str::from_utf8(bytes).map_err(|_| Error::Utf8Error)?;
    let num = u32::from_str_radix(string, 16).map_err(|_| Error::ParseIntError)?;
    Ok(num)
}

fn align_up_pad(size: usize, align: usize) -> usize {
    align_up(size, align) - size
}

fn align_up(size: usize, align: usize) -> usize {
    debug_assert!(align >= 2 && align.is_power_of_two());
    (size + align - 1) & !(align - 1)
}
