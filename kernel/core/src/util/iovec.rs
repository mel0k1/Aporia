use ostd::{
    Error as OstdError,
    mm::{self, Infallible, VmSpace},
};

use crate::prelude::*;

#[derive(Clone, Copy, Debug)]
struct IoVec {
    base: Vaddr,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct UserIoVec {
    pub base: Vaddr,
    pub len: isize,
}

impl TryFrom<UserIoVec> for IoVec {
    type Error = Error;

    fn try_from(value: UserIoVec) -> Result<Self> {
        if value.len < 0 {
            return Err(errno!(EINVAL, "the I/O buffer length cannot be negative"));
        }

        Ok(IoVec {
            base: value.base,
            len: value.len as usize,
        })
    }
}

impl IoVec {
    const fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn reader<'a>(&self, vm_space: &'a VmSpace) -> Result<VmReader<'a>> {
        Ok(vm_space.reader(self.base, self.len)?)
    }

    fn writer<'a>(&self, vm_space: &'a VmSpace) -> Result<VmWriter<'a>> {
        Ok(vm_space.writer(self.base, self.len)?)
    }
}

pub(super) const MAX_IO_VECTOR_LENGTH: usize = 1024;
const MAX_TOTAL_IOV_BYTES: usize = isize::MAX as usize;

fn copy_iovs_and_convert<'a, T: 'a>(
    user_space: &'a CurrentUserSpace<'a>,
    start_addr: Vaddr,
    count: usize,
    convert_iovec: impl Fn(&IoVec, &'a VmSpace) -> Result<T>,
) -> Result<Box<[T]>> {
    if count > MAX_IO_VECTOR_LENGTH {
        return Err(errno!(EINVAL, "the I/O vector contains too many buffers"));
    }

    let vm_space = user_space.vmar().vm_space();

    let mut iovs = Vec::with_capacity(count);
    for idx in 0..count {
        let addr = start_addr + idx * size_of::<UserIoVec>();
        let uiov: UserIoVec = vm_space.reader(addr, size_of::<UserIoVec>())?.read_val()?;
        iovs.push(IoVec::try_from(uiov)?);
    }

    let mut v = Vec::with_capacity(count);
    let mut max_len = MAX_TOTAL_IOV_BYTES;
    for mut iov in iovs {
        if !mm::is_in_user_space(iov.base, iov.len) {
            return Err(errno!(EFAULT, "the I/O buffer range is outside user space"));
        }

        if iov.len > max_len {
            iov.len = max_len;
        }
        max_len -= iov.len;

        if iov.is_empty() {
            continue;
        }

        let converted = convert_iovec(&iov, vm_space)?;
        v.push(converted)
    }

    Ok(v.into_boxed_slice())
}

pub(crate) struct VmReaderArray<'a>(Box<[VmReader<'a>]>);

pub(crate) struct VmWriterArray<'a>(Box<[VmWriter<'a>]>);

impl<'a> VmReaderArray<'a> {
    pub(crate) fn from_user_io_vecs(
        user_space: &'a CurrentUserSpace<'a>,
        start_addr: Vaddr,
        count: usize,
    ) -> Result<Self> {
        let readers = copy_iovs_and_convert(user_space, start_addr, count, IoVec::reader)?;
        Ok(Self(readers))
    }

    pub(crate) fn readers_mut(&mut self) -> &mut [VmReader<'a>] {
        &mut self.0
    }

    #[cfg(ktest)]
    pub(crate) const fn new(readers: Box<[VmReader<'a>]>) -> Self {
        Self(readers)
    }
}

impl<'a> VmWriterArray<'a> {
    pub(crate) fn from_user_io_vecs(
        user_space: &'a CurrentUserSpace<'a>,
        start_addr: Vaddr,
        count: usize,
    ) -> Result<Self> {
        let writers = copy_iovs_and_convert(user_space, start_addr, count, IoVec::writer)?;
        Ok(Self(writers))
    }

    pub(crate) fn writers_mut(&mut self) -> &mut [VmWriter<'a>] {
        &mut self.0
    }
}

pub(crate) trait MultiRead: ReadCString {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)>;

    fn sum_lens(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    fn skip_some(&mut self, nbytes: usize);
}

pub(crate) trait MultiWrite {
    fn write(&mut self, reader: &mut VmReader<'_, Infallible>)
    -> Result<usize, (OstdError, usize)>;

    fn sum_lens(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.sum_lens() == 0
    }

    fn skip_some(&mut self, nbytes: usize);
}

impl MultiRead for VmReaderArray<'_> {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)> {
        let mut total_len = 0;

        for reader in &mut self.0 {
            let copied_len = reader
                .read_fallible(writer)
                .map_err(|(err, copied_len)| (err, total_len + copied_len))?;
            total_len += copied_len;
            if !writer.has_avail() {
                break;
            }
        }
        Ok(total_len)
    }

    fn sum_lens(&self) -> usize {
        self.0.iter().map(|vm_reader| vm_reader.remain()).sum()
    }

    fn skip_some(&mut self, mut nbytes: usize) {
        for reader in &mut self.0 {
            let bytes_to_skip = reader.remain().min(nbytes);
            reader.skip(bytes_to_skip);
            nbytes -= bytes_to_skip;

            if nbytes == 0 {
                return;
            }
        }
    }
}

impl MultiRead for VmReader<'_> {
    fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> Result<usize, (OstdError, usize)> {
        self.read_fallible(writer)
    }

    fn sum_lens(&self) -> usize {
        self.remain()
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.remain().min(nbytes));
    }
}

impl dyn MultiRead + '_ {
    pub(crate) fn read_val_opt<T: Pod>(&mut self) -> Result<Option<T>> {
        let mut val = T::new_zeroed();
        let nbytes = self.read(&mut VmWriter::from(val.as_mut_bytes()))?;

        if nbytes == size_of::<T>() {
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }
}

impl MultiWrite for VmWriterArray<'_> {
    fn write(
        &mut self,
        reader: &mut VmReader<'_, Infallible>,
    ) -> Result<usize, (OstdError, usize)> {
        let mut total_len = 0;

        for writer in &mut self.0 {
            let copied_len = writer
                .write_fallible(reader)
                .map_err(|(err, copied_len)| (err, total_len + copied_len))?;
            total_len += copied_len;
            if !reader.has_remain() {
                break;
            }
        }
        Ok(total_len)
    }

    fn sum_lens(&self) -> usize {
        self.0.iter().map(|vm_writer| vm_writer.avail()).sum()
    }

    fn skip_some(&mut self, mut nbytes: usize) {
        for writer in &mut self.0 {
            let bytes_to_skip = writer.avail().min(nbytes);
            writer.skip(bytes_to_skip);
            nbytes -= bytes_to_skip;

            if nbytes == 0 {
                return;
            }
        }
    }
}

impl MultiWrite for VmWriter<'_> {
    fn write(
        &mut self,
        reader: &mut VmReader<'_, Infallible>,
    ) -> Result<usize, (OstdError, usize)> {
        self.write_fallible(reader)
    }

    fn sum_lens(&self) -> usize {
        self.avail()
    }

    fn skip_some(&mut self, nbytes: usize) {
        self.skip(self.avail().min(nbytes));
    }
}

impl dyn MultiWrite + '_ {
    pub(crate) fn write_val_trunc<T: Pod>(&mut self, val: &T) -> Result<()> {
        let _nbytes = self.write(&mut VmReader::from(val.as_bytes()))?;

        Ok(())
    }
}
