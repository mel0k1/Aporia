use core::{fmt::Debug, marker::PhantomData};

use aster_rights::{Dup, Exec, Full, Read, Signal, TRightSet, TRights, Write};
use aster_rights_proc::require;
use ostd::{
    Result,
    mm::{Daddr, HasDaddr, HasPaddr, Paddr, PodOnce, VmIo, VmIoOnce, dma::DmaDirection},
};
use ostd_pod::Pod;

pub struct SafePtr<T, M, R = Full> {
    offset: usize,
    vm_obj: M,
    rights: R,
    phantom: PhantomData<T>,
}

impl<T, M> SafePtr<T, M> {
    pub fn new(vm_obj: M, offset: usize) -> Self {
        Self {
            offset,
            vm_obj,
            rights: TRightSet(<TRights![Dup, Read, Write, Exec, Signal]>::new()),
            phantom: PhantomData,
        }
    }
}

impl<T, M: HasPaddr, R> HasPaddr for SafePtr<T, M, R> {
    fn paddr(&self) -> Paddr {
        self.vm_obj.paddr() + self.offset
    }
}

impl<T: Pod, M: VmIo, R: TRights> SafePtr<T, M, TRightSet<R>> {
    #[require(R > Read)]
    pub fn read(&self) -> Result<T> {
        self.vm_obj.read_val(self.offset)
    }

    #[require(R > Read)]
    pub fn read_slice(&self, slice: &mut [T]) -> Result<()> {
        self.vm_obj.read_slice(self.offset, slice)
    }

    #[require(R > Write)]
    pub fn write(&self, val: &T) -> Result<()> {
        self.vm_obj.write_val(self.offset, val)
    }

    #[require(R > Write)]
    pub fn write_slice(&self, slice: &[T]) -> Result<()> {
        self.vm_obj.write_slice(self.offset, slice)
    }

    #[require(R > Write)]
    #[require(R1 > Read)]
    pub fn copy_from<M1: VmIo, R1: TRights>(
        &self,
        ptr: &SafePtr<T, M1, TRightSet<R1>>,
    ) -> Result<()> {
        let val = ptr.vm_obj.read_val::<T>(ptr.offset)?;
        self.vm_obj.write_val(self.offset, &val)
    }
}

impl<T: PodOnce, M: VmIoOnce, R: TRights> SafePtr<T, M, TRightSet<R>> {
    #[require(R > Read)]
    pub fn read_once(&self) -> Result<T> {
        self.vm_obj.read_once(self.offset)
    }

    #[require(R > Write)]
    pub fn write_once(&self, val: &T) -> Result<()> {
        self.vm_obj.write_once(self.offset, val)
    }
}

impl<T, M, R> SafePtr<T, M, R> {
    pub const fn is_aligned(&self) -> bool {
        self.offset.is_multiple_of(align_of::<T>())
    }

    pub fn add(&mut self, count: usize) {
        let offset = count * size_of::<T>();
        self.offset += offset;
    }

    pub fn offset(&mut self, count: isize) {
        let offset = count * size_of::<T>() as isize;
        if count >= 0 {
            self.offset += offset as usize;
        } else {
            self.offset -= offset as usize;
        }
    }

    pub fn byte_add(&mut self, bytes: usize) {
        self.offset += bytes;
    }

    pub fn byte_offset(&mut self, bytes: isize) {
        if bytes >= 0 {
            self.offset += bytes as usize;
        } else {
            self.offset -= (-bytes) as usize;
        }
    }
}

impl<T, M, R> SafePtr<T, M, R> {
    pub const fn vm(&self) -> &M {
        &self.vm_obj
    }

    pub fn set_vm(&mut self, vm_obj: M) {
        self.vm_obj = vm_obj;
    }
}

impl<T, M, R: Clone> SafePtr<T, M, R> {
    pub fn borrow_vm(&self) -> SafePtr<T, &M, R> {
        let SafePtr {
            offset: addr,
            vm_obj,
            rights,
            ..
        } = self;
        SafePtr {
            offset: *addr,
            vm_obj,
            rights: rights.clone(),
            phantom: PhantomData,
        }
    }
}

impl<T, M, R> SafePtr<T, M, R> {
    pub fn cast<U>(self) -> SafePtr<U, M, R> {
        let SafePtr {
            offset: addr,
            vm_obj,
            rights,
            ..
        } = self;
        SafePtr {
            offset: addr,
            vm_obj,
            rights,
            phantom: PhantomData,
        }
    }
}

impl<T, M, R: TRights> SafePtr<T, M, TRightSet<R>> {
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> SafePtr<T, M, TRightSet<R1>> {
        let SafePtr {
            offset: addr,
            vm_obj,
            ..
        } = self;
        SafePtr {
            offset: addr,
            vm_obj,
            rights: TRightSet(R1::new()),
            phantom: PhantomData,
        }
    }
}

impl<T, M: HasDaddr, R> HasDaddr for SafePtr<T, M, R> {
    fn daddr(&self) -> Daddr {
        self.offset + self.vm_obj.daddr()
    }
}

impl<T, R, D: DmaDirection> SafePtr<T, ostd::mm::dma::DmaStream<D>, R> {
    pub fn sync_from_device(&self) -> Result<()> {
        self.vm_obj
            .sync_from_device(self.offset..self.offset + size_of::<T>())
    }

    pub fn sync_to_device(&self) -> Result<()> {
        self.vm_obj
            .sync_to_device(self.offset..self.offset + size_of::<T>())
    }
}

#[require(R > Dup)]
impl<T, M: Clone, R: TRights> Clone for SafePtr<T, M, TRightSet<R>> {
    fn clone(&self) -> Self {
        Self {
            offset: self.offset,
            vm_obj: self.vm_obj.clone(),
            rights: self.rights,
            phantom: PhantomData,
        }
    }
}

#[require(R > Dup)]
impl<T, M: crate::dup::Dup, R: TRights> crate::dup::Dup for SafePtr<T, M, TRightSet<R>> {
    fn dup(&self) -> Result<Self> {
        let duplicated = Self {
            offset: self.offset,
            vm_obj: self.vm_obj.dup()?,
            rights: self.rights,
            phantom: PhantomData,
        };
        Ok(duplicated)
    }
}

impl<T, M: Debug, R> Debug for SafePtr<T, M, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SafePtr")
            .field("offset", &self.offset)
            .field("vm_obj", &self.vm_obj)
            .finish()
    }
}

#[macro_export]
macro_rules! field_ptr {
    ($ptr:expr, $type:ty, $($field:tt)+) => {{
        use aster_util::safe_ptr::SafePtr;

        #[inline]
        fn new_field_ptr<T, M, R: Clone, U>(
            container_ptr: &SafePtr<T, M, R>,
            field_offset: usize,
            _type_infer: *const U,
        ) -> SafePtr<U, &M, R>
        {
            let mut ptr = container_ptr.borrow_vm();
            ptr.byte_add(field_offset);
            ptr.cast()
        }

        let field_offset = core::mem::offset_of!($type, $($field)*);
        let type_infer = ostd::ptr_null_of!({
            let ptr: *const $type = core::ptr::null();
            &raw const (*ptr).$($field)*
        });

        new_field_ptr($ptr, field_offset, type_infer)
    }}
}
