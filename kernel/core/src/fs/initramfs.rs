macro_rules! __log_prefix {
    () => {
        "initramfs: "
    };
}

use alloc::{
    borrow::ToOwned,
    io::{self, Cursor, Read},
};

use cpio_decoder::{CpioDecoder, CpioEntry, FileMetadata, FileType};
use device_id::{DeviceId, MajorId, MinorId};
use lending_iterator::LendingIterator;
use miniz_oxide::{
    DataFormat, MZFlush, MZStatus,
    inflate::stream::{InflateState, inflate},
};
use ostd::boot::boot_info;
use spin::once::Once;

use super::{
    file::{InodeMode, InodeType, mkmod},
    vfs::path::{FsPath, Path, PathResolver, is_dot},
};
use crate::{
    device::tty,
    fs::{
        file::StatusFlags,
        vfs::inode::{Inode, MknodType},
    },
    prelude::*,
};

pub(crate) fn init_in_first_kthread(path_resolver: &PathResolver) -> Result<()> {
    let Some(initramfs_buf) = boot_info().initramfs else {
        return Ok(());
    };

    match &initramfs_buf[..4] {
        &[0x1F, 0x8B, _, _] => {
            println!("[kernel] unpacking initramfs.cpio.gz to rootfs ...");
            unpack_to_rootfs(
                CpioDecoder::new(GzipReader::new(initramfs_buf)),
                path_resolver,
            )?;
        }
        _ => {
            println!("[kernel] unpacking initramfs.cpio to rootfs ...");
            unpack_to_rootfs(CpioDecoder::new(Cursor::new(initramfs_buf)), path_resolver)?;
        }
    };

    ensure_dev_console(path_resolver)?;

    println!("[kernel] initramfs is ready");
    Ok(())
}

fn unpack_to_rootfs<R: Read>(
    mut decoder: CpioDecoder<R>,
    path_resolver: &PathResolver,
) -> Result<()> {
    while let Some(entry_result) = decoder.next() {
        let mut entry = entry_result?;
        if let Err(e) = try_append_entry_to_rootfs(&mut entry, path_resolver) {
            warn!("failed to add entry {} to rootfs: {:?}", entry.name(), e);
        }
    }
    Ok(())
}

fn ensure_dev_console(path_resolver: &PathResolver) -> Result<()> {
    let dev_path = super::lookup_or_create_dev(path_resolver)?;
    match path_resolver.lookup(&FsPath::try_from("/dev/console")?) {
        Ok(_) => Ok(()),
        Err(error) if error.error() == Errno::ENOENT => {
            dev_path.mknod(
                "console",
                mkmod!(u+rw),
                MknodType::CharDevice(tty::CONSOLE_DEVICE_ID.as_encoded_u64()),
            )?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn find_init(path_resolver: &PathResolver) -> Result<(Path, &'static str)> {
    const DEFAULT_INITRAMFS_INIT_PATH: &str = "/init";

    let init_path = RDINIT_PATH
        .get()
        .map(String::as_str)
        .unwrap_or(DEFAULT_INITRAMFS_INIT_PATH);
    let path = path_resolver.lookup(&FsPath::try_from(init_path)?)?;
    Ok((path, init_path))
}

fn try_append_entry_to_rootfs<R: Read>(
    entry: &mut CpioEntry<R>,
    path_resolver: &PathResolver,
) -> Result<()> {
    let entry_name = entry.name().trim_start_matches('/').trim_end_matches('/');
    if entry_name.is_empty() {
        return Err(errno!(EINVAL, "invalid entry name"));
    }
    if is_dot(entry_name) {
        return Ok(());
    }

    let (parent, name) = if let Some((prefix, last)) = entry_name.rsplit_once('/') {
        (path_resolver.lookup(&FsPath::try_from(prefix)?)?, last)
    } else {
        (path_resolver.root().clone(), entry_name)
    };

    let metadata = entry.metadata();
    let mode = InodeMode::from_bits_truncate(metadata.permission_mode());
    match metadata.file_type() {
        FileType::File => {
            let path = parent.new_child(name, InodeType::File, mode)?;
            let writer = InodeWriter {
                inner: path.inode().as_ref(),
                offset: 0,
            };
            entry.read_all(writer)?;
        }
        FileType::Dir => {
            let _ = parent.new_child(name, InodeType::Dir, mode)?;
        }
        FileType::Link => {
            let child_name = name.to_owned();

            let mut link_data: Vec<u8> = Vec::new();
            entry.read_all(&mut link_data)?;
            let link_content = core::str::from_utf8(&link_data)?;

            parent.new_symlink_child(&child_name, link_content, mode)?;
        }
        FileType::Char => {
            let device_id = try_device_id_from_metadata(metadata)?;
            parent.mknod(name, mode, MknodType::CharDevice(device_id))?;
        }
        FileType::Block => {
            let device_id = try_device_id_from_metadata(metadata)?;
            parent.mknod(name, mode, MknodType::BlockDevice(device_id))?;
        }
        FileType::FiFo => {
            parent.mknod(name, mode, MknodType::NamedPipe)?;
        }
        FileType::Socket => {
            return Err(errno!(
                EINVAL,
                "socket files are not supported in initramfs"
            ));
        }
    }

    Ok(())
}

struct GzipReader<'a> {
    deflate_body: &'a [u8],
    state: Box<InflateState>,
    done: bool,
}

impl<'a> GzipReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            deflate_body: strip_gzip_header(buf),
            state: InflateState::new_boxed(DataFormat::Raw),
            done: false,
        }
    }
}

impl Read for GzipReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        while !self.done {
            let result = inflate(&mut self.state, self.deflate_body, out, MZFlush::None);
            self.deflate_body = &self.deflate_body[result.bytes_consumed..];
            match result.status {
                Ok(MZStatus::StreamEnd) => self.done = true,
                Ok(_) => {}
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "gzip decompression failed",
                    ));
                }
            }
            if result.bytes_written > 0 {
                return Ok(result.bytes_written);
            }
            if result.bytes_consumed == 0 {
                break;
            }
        }

        Ok(0)
    }
}

fn strip_gzip_header(buf: &[u8]) -> &[u8] {
    const FLG_FHCRC: u8 = 0x02;
    const FLG_FEXTRA: u8 = 0x04;
    const FLG_FNAME: u8 = 0x08;
    const FLG_FCOMMENT: u8 = 0x10;

    let flg = buf[3];
    let mut pos = 10;

    if flg & FLG_FEXTRA != 0 {
        let xlen = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2 + xlen;
    }
    if flg & FLG_FNAME != 0 {
        pos += buf[pos..].iter().position(|&b| b == 0).unwrap() + 1;
    }
    if flg & FLG_FCOMMENT != 0 {
        pos += buf[pos..].iter().position(|&b| b == 0).unwrap() + 1;
    }
    if flg & FLG_FHCRC != 0 {
        pos += 2;
    }

    &buf[pos..]
}

struct InodeWriter<'a> {
    inner: &'a dyn Inode,
    offset: usize,
}

impl io::Write for InodeWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        let write_len = self
            .inner
            .write_at(self.offset, &mut reader, StatusFlags::empty())
            .map_err(|_| io::ErrorKind::WriteZero)?;
        self.offset += write_len;
        Ok(write_len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn try_device_id_from_metadata(metadata: &FileMetadata) -> Result<u64> {
    let major = {
        let dev_maj = u16::try_from(metadata.rdev_maj())?;
        MajorId::try_from(dev_maj).map_err(|_msg| errno!(EINVAL, _msg))?
    };
    let minor = MinorId::try_from(metadata.rdev_min()).map_err(|_msg| errno!(EINVAL, _msg))?;
    Ok(DeviceId::new(major, minor).as_encoded_u64())
}

static RDINIT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("rdinit", RDINIT_PATH);
