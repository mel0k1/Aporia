pub(crate) mod copy;
pub mod util;

use core::{marker::PhantomData, mem::MaybeUninit};

use ostd_pod::Pod;

use self::copy::{memcpy, memset};
use crate::{
    Error,
    arch::mm::{__atomic_cmpxchg_fallible, __atomic_load_fallible},
    mm::kspace::{KERNEL_BASE_VADDR, KERNEL_END_VADDR},
    prelude::*,
};

pub trait VmIo {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()>;

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read(offset, &mut writer)
    }

    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut val = T::new_zeroed();
        self.read_bytes(offset, val.as_mut_bytes())?;
        Ok(val)
    }

    fn read_slice<T: Pod>(&self, offset: usize, slice: &mut [T]) -> Result<()> {
        let len_in_bytes = size_of_val(slice);
        let ptr = slice as *mut [T] as *mut u8;
        let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len_in_bytes) };
        self.read_bytes(offset, buf)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()>;

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write(offset, &mut reader)
    }

    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.write_bytes(offset, new_val.as_bytes())?;
        Ok(())
    }

    fn write_slice<T: Pod>(&self, offset: usize, slice: &[T]) -> Result<()> {
        let len_in_bytes = size_of_val(slice);
        let ptr = slice as *const [T] as *const u8;
        let buf = unsafe { core::slice::from_raw_parts(ptr, len_in_bytes) };
        self.write_bytes(offset, buf)
    }
}

pub trait VmIoFill {
    fn fill_zeros(&self, offset: usize, len: usize) -> Result<(), (Error, usize)>;
}

pub trait VmIoOnce {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T>;

    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()>;
}

pub enum Fallible {}

pub enum Infallible {}

pub(crate) enum Io {}

pub trait FallibleVmRead<F> {
    fn read_fallible(&mut self, writer: &mut VmWriter<'_, F>) -> Result<usize, (Error, usize)>;
}

pub trait FallibleVmWrite<F> {
    fn write_fallible(&mut self, reader: &mut VmReader<'_, F>) -> Result<usize, (Error, usize)>;
}

pub struct VmReader<'a, Fallibility = Fallible> {
    cursor: *const u8,
    end: *const u8,
    phantom: PhantomData<(&'a [u8], Fallibility)>,
}

impl<Fallibility> Clone for VmReader<'_, Fallibility> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
    }
}

macro_rules! impl_read_fallible {
    ($reader_fallibility:ty, $writer_fallibility:ty) => {
        impl<'a> FallibleVmRead<$writer_fallibility> for VmReader<'a, $reader_fallibility> {
            fn read_fallible(
                &mut self,
                writer: &mut VmWriter<'_, $writer_fallibility>,
            ) -> Result<usize, (Error, usize)> {
                let copy_len = self.remain().min(writer.avail());
                if copy_len == 0 {
                    return Ok(0);
                }

                let copied_len = unsafe {
                    memcpy::<$writer_fallibility, $reader_fallibility>(
                        writer.cursor,
                        self.cursor,
                        copy_len,
                    )
                };
                self.cursor = self.cursor.wrapping_add(copied_len);
                writer.cursor = writer.cursor.wrapping_add(copied_len);

                if copied_len < copy_len {
                    Err((Error::PageFault, copied_len))
                } else {
                    Ok(copied_len)
                }
            }
        }
    };
}

macro_rules! impl_write_fallible {
    ($writer_fallibility:ty, $reader_fallibility:ty) => {
        impl<'a> FallibleVmWrite<$reader_fallibility> for VmWriter<'a, $writer_fallibility> {
            fn write_fallible(
                &mut self,
                reader: &mut VmReader<'_, $reader_fallibility>,
            ) -> Result<usize, (Error, usize)> {
                reader.read_fallible(self)
            }
        }
    };
}

impl_read_fallible!(Fallible, Infallible);
impl_read_fallible!(Fallible, Fallible);
impl_read_fallible!(Infallible, Fallible);
impl_write_fallible!(Fallible, Infallible);
impl_write_fallible!(Fallible, Fallible);
impl_write_fallible!(Infallible, Fallible);

impl<'a> VmReader<'a, Infallible> {
    pub unsafe fn from_kernel_space(ptr: *const u8, len: usize) -> Self {
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr.addr());
        debug_assert!(len == 0 || ptr.addr().checked_add(len).unwrap() <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
            phantom: PhantomData,
        }
    }

    pub fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> usize {
        let copy_len = self.remain().min(writer.avail());
        if copy_len == 0 {
            return 0;
        }

        unsafe { memcpy::<Infallible, Infallible>(writer.cursor, self.cursor, copy_len) };
        self.cursor = self.cursor.wrapping_add(copy_len);
        writer.cursor = writer.cursor.wrapping_add(copy_len);

        copy_len
    }

    pub fn read_val<T: Pod>(&mut self) -> Result<T> {
        if self.remain() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();

        let val = unsafe { core::intrinsics::unaligned_volatile_load(cursor) };
        self.cursor = self.cursor.wrapping_add(size_of::<T>());

        Ok(val)
    }

    pub fn read_once<T: PodOnce>(&mut self) -> Result<T> {
        if self.remain() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        const { assert!(pod_once_impls::is_non_tearing::<T>()) };

        let val = unsafe { cursor.read_volatile() };
        self.cursor = self.cursor.wrapping_add(size_of::<T>());

        Ok(val)
    }

    pub fn to_fallible(self) -> VmReader<'a, Fallible> {
        VmReader {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
    }
}

impl VmReader<'_, Fallible> {
    pub unsafe fn from_user_space(ptr: *const u8, len: usize) -> Self {
        debug_assert!(super::is_in_user_space(ptr.addr(), len));

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
            phantom: PhantomData,
        }
    }

    pub fn read_val<T: Pod>(&mut self) -> Result<T> {
        if self.remain() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut val = MaybeUninit::<T>::uninit();

        let mut writer =
            unsafe { VmWriter::from_kernel_space(val.as_mut_ptr().cast(), size_of::<T>()) };
        self.read_fallible(&mut writer)
            .map_err(|(err, copied_len)| {
                self.cursor = self.cursor.wrapping_sub(copied_len);
                err
            })?;
        debug_assert!(!writer.has_avail());

        let val_inited = unsafe { val.assume_init() };
        Ok(val_inited)
    }

    pub fn atomic_load<T: PodAtomic>(&self) -> Result<T> {
        if self.remain() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        unsafe { T::atomic_load_fallible(cursor) }
    }
}

impl<Fallibility> VmReader<'_, Fallibility> {
    pub fn remain(&self) -> usize {
        self.end.addr() - self.cursor.addr()
    }

    pub fn cursor(&self) -> *const u8 {
        self.cursor
    }

    pub fn has_remain(&self) -> bool {
        self.remain() > 0
    }

    pub fn limit(&mut self, max_remain: usize) -> &mut Self {
        if max_remain < self.remain() {
            self.end = self.cursor.wrapping_add(max_remain);
        }

        self
    }

    pub fn skip(&mut self, nbytes: usize) -> &mut Self {
        assert!(nbytes <= self.remain());
        self.cursor = self.cursor.wrapping_add(nbytes);

        self
    }
}

impl<'a> From<&'a [u8]> for VmReader<'a, Infallible> {
    fn from(slice: &'a [u8]) -> Self {
        unsafe { Self::from_kernel_space(slice.as_ptr(), slice.len()) }
    }
}

pub struct VmWriter<'a, Fallibility = Fallible> {
    cursor: *mut u8,
    end: *mut u8,
    phantom: PhantomData<(&'a mut [u8], Fallibility)>,
}

impl<'a> VmWriter<'a, Infallible> {
    pub unsafe fn from_kernel_space(ptr: *mut u8, len: usize) -> Self {
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr.addr());
        debug_assert!(len == 0 || ptr.addr().checked_add(len).unwrap() <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
            phantom: PhantomData,
        }
    }

    pub fn write(&mut self, reader: &mut VmReader<'_, Infallible>) -> usize {
        reader.read(self)
    }

    pub fn write_val<T: Pod>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();

        unsafe { core::intrinsics::unaligned_volatile_store(cursor, *new_val) };
        self.cursor = self.cursor.wrapping_add(size_of::<T>());

        Ok(())
    }

    pub fn write_once<T: PodOnce>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        const { assert!(pod_once_impls::is_non_tearing::<T>()) };

        unsafe { cursor.write_volatile(*new_val) };
        self.cursor = self.cursor.wrapping_add(size_of::<T>());

        Ok(())
    }

    pub fn fill_zeros(&mut self, len: usize) -> usize {
        let len_to_set = self.avail().min(len);
        if len_to_set == 0 {
            return 0;
        }

        unsafe { memset::<Infallible>(self.cursor, 0u8, len_to_set) };
        self.cursor = self.cursor.wrapping_add(len_to_set);

        len_to_set
    }

    pub fn to_fallible(self) -> VmWriter<'a, Fallible> {
        VmWriter {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
    }
}

impl VmWriter<'_, Fallible> {
    pub unsafe fn from_user_space(ptr: *mut u8, len: usize) -> Self {
        debug_assert!(super::is_in_user_space(ptr.addr(), len));

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
            phantom: PhantomData,
        }
    }

    pub fn write_val<T: Pod>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut reader = VmReader::from(new_val.as_bytes());
        self.write_fallible(&mut reader)
            .map_err(|(err, copied_len)| {
                self.cursor = self.cursor.wrapping_sub(copied_len);
                err
            })?;
        Ok(())
    }

    pub fn atomic_compare_exchange<T>(
        &self,
        reader: &VmReader,
        old_val: T,
        new_val: T,
    ) -> Result<(T, bool)>
    where
        T: PodAtomic + Eq,
    {
        if self.avail() < size_of::<T>() || reader.remain() < size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        assert_eq!(self.cursor.cast_const(), reader.cursor);

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        let cur_val = unsafe { T::atomic_cmpxchg_fallible(cursor, old_val, new_val)? };

        Ok((cur_val, old_val == cur_val))
    }

    pub fn fill_zeros(&mut self, len: usize) -> Result<usize, (Error, usize)> {
        let len_to_set = self.avail().min(len);
        if len_to_set == 0 {
            return Ok(0);
        }

        let set_len = unsafe { memset::<Fallible>(self.cursor, 0u8, len_to_set) };
        self.cursor = self.cursor.wrapping_add(set_len);

        if set_len < len_to_set {
            Err((Error::PageFault, set_len))
        } else {
            Ok(len_to_set)
        }
    }
}

impl<Fallibility> VmWriter<'_, Fallibility> {
    pub fn avail(&self) -> usize {
        self.end.addr() - self.cursor.addr()
    }

    pub fn cursor(&self) -> *mut u8 {
        self.cursor
    }

    pub fn has_avail(&self) -> bool {
        self.avail() > 0
    }

    pub fn limit(&mut self, max_avail: usize) -> &mut Self {
        if max_avail < self.avail() {
            self.end = self.cursor.wrapping_add(max_avail);
        }

        self
    }

    pub fn skip(&mut self, nbytes: usize) -> &mut Self {
        assert!(nbytes <= self.avail());
        self.cursor = self.cursor.wrapping_add(nbytes);

        self
    }

    pub fn clone_exclusive(&mut self) -> VmWriter<'_, Fallibility> {
        VmWriter {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
    }
}

impl<'a> From<&'a mut [u8]> for VmWriter<'a, Infallible> {
    fn from(slice: &'a mut [u8]) -> Self {
        unsafe { Self::from_kernel_space(slice.as_mut_ptr(), slice.len()) }
    }
}

pub trait PodOnce: Pod {}

#[cfg(any(
    target_arch = "x86_64",
    target_arch = "riscv64",
    target_arch = "loongarch64",
    target_arch = "aarch64"
))]
mod pod_once_impls {
    use super::PodOnce;

    impl PodOnce for u8 {}
    impl PodOnce for u16 {}
    impl PodOnce for u32 {}
    impl PodOnce for u64 {}
    impl PodOnce for usize {}
    impl PodOnce for i8 {}
    impl PodOnce for i16 {}
    impl PodOnce for i32 {}
    impl PodOnce for i64 {}
    impl PodOnce for isize {}

    pub(super) const fn is_non_tearing<T>() -> bool {
        let size = size_of::<T>();

        size == 1 || size == 2 || size == 4 || size == 8
    }
}

pub trait PodAtomic: Pod {
    #[doc(hidden)]
    unsafe fn atomic_load_fallible(ptr: *const Self) -> Result<Self>;

    #[doc(hidden)]
    unsafe fn atomic_cmpxchg_fallible(ptr: *mut Self, old_val: Self, new_val: Self)
    -> Result<Self>;
}

impl PodAtomic for u32 {
    unsafe fn atomic_load_fallible(ptr: *const Self) -> Result<Self> {
        let result = unsafe { __atomic_load_fallible(ptr) };
        if result == !0 {
            Err(Error::PageFault)
        } else {
            Ok(result as Self)
        }
    }

    unsafe fn atomic_cmpxchg_fallible(ptr: *mut Self, old_val: Self, new_val: Self) -> Result<u32> {
        let result = unsafe { __atomic_cmpxchg_fallible(ptr, old_val, new_val) };
        if result == !0 {
            Err(Error::PageFault)
        } else {
            Ok(result as Self)
        }
    }
}
