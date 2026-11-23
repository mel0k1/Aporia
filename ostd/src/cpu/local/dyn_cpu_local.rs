use core::{marker::PhantomData, mem::ManuallyDrop, ptr::NonNull};

use bitvec::prelude::{BitVec, bitvec};

use super::{AnyStorage, CpuLocal};
use crate::{
    Result,
    cpu::{CpuId, PinCurrentCpu, all_cpus, num_cpus},
    irq::DisabledLocalIrqGuard,
    mm::{FrameAllocOptions, HasPaddr, PAGE_SIZE, Segment, Vaddr, paddr_to_vaddr},
    util::id_set::Id,
};

pub struct DynamicStorage<T>(NonNull<T>);

unsafe impl<T> AnyStorage<T> for DynamicStorage<T> {
    fn get_ptr_on_current(&self, guard: &DisabledLocalIrqGuard) -> *const T {
        self.get_ptr_on_target(guard.current_cpu())
    }

    fn get_ptr_on_target(&self, cpu_id: CpuId) -> *const T {
        let bsp_va = self.0.as_ptr() as usize;
        let va = bsp_va + cpu_id.as_usize() * CHUNK_SIZE;
        va as *mut T
    }
}

impl<T> DynamicStorage<T> {
    fn get_mut_ptr_on_target(&mut self, cpu: CpuId) -> *mut T {
        self.get_ptr_on_target(cpu).cast_mut()
    }
}

impl<T> Drop for DynamicStorage<T> {
    fn drop(&mut self) {
        panic!(
            "Do not drop `DynamicStorage<T>` directly. \
            Use `DynCpuLocalChunk::try_dealloc<T>` instead."
        );
    }
}

impl<T: Sync + alloc::fmt::Debug + 'static> alloc::fmt::Debug for CpuLocal<T, DynamicStorage<T>> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut list = f.debug_list();
        for cpu in all_cpus() {
            let val = self.get_on_cpu(cpu);
            list.entry(&(&cpu, val));
        }
        list.finish()
    }
}

impl<T> CpuLocal<T, DynamicStorage<T>> {
    unsafe fn __new_dynamic(ptr: *mut T, init_values: &mut impl FnMut(CpuId) -> T) -> Self {
        let mut storage = DynamicStorage(NonNull::new(ptr).unwrap());
        for cpu in all_cpus() {
            let ptr = storage.get_mut_ptr_on_target(cpu);
            unsafe {
                core::ptr::write(ptr, init_values(cpu));
            }
        }

        Self {
            storage,
            phantom: PhantomData,
        }
    }
}

const CHUNK_SIZE: usize = PAGE_SIZE;

#[derive(Clone, Copy, Debug)]
struct DynCpuLocalMeta;
crate::impl_frame_meta_for!(DynCpuLocalMeta);

pub struct DynCpuLocalChunk<const ITEM_SIZE: usize> {
    segment: ManuallyDrop<Segment<DynCpuLocalMeta>>,
    bitmap: BitVec,
}

impl<const ITEM_SIZE: usize> DynCpuLocalChunk<ITEM_SIZE> {
    pub fn new() -> Result<Self> {
        let total_chunk_size = CHUNK_SIZE * num_cpus();
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(total_chunk_size.div_ceil(PAGE_SIZE), |_| DynCpuLocalMeta)?;

        let num_items = CHUNK_SIZE / ITEM_SIZE;
        const { assert!(CHUNK_SIZE.is_multiple_of(ITEM_SIZE)) };

        Ok(Self {
            segment: ManuallyDrop::new(segment),
            bitmap: bitvec![0; num_items],
        })
    }

    fn start_vaddr(&self) -> Vaddr {
        paddr_to_vaddr(self.segment.paddr())
    }

    pub fn alloc<T>(
        &mut self,
        init_values: &mut impl FnMut(CpuId) -> T,
    ) -> Option<CpuLocal<T, DynamicStorage<T>>> {
        assert!(ITEM_SIZE.is_power_of_two());
        assert!(size_of::<T>() <= ITEM_SIZE);
        assert!(align_of::<T>() <= ITEM_SIZE);

        let index = self.bitmap.first_zero()?;
        self.bitmap.set(index, true);
        unsafe {
            let vaddr = self.start_vaddr() + index * ITEM_SIZE;
            Some(CpuLocal::__new_dynamic(vaddr as *mut T, init_values))
        }
    }

    fn get_item_index<T>(&mut self, cpu_local: &CpuLocal<T, DynamicStorage<T>>) -> Option<usize> {
        let vaddr = cpu_local.storage.0.as_ptr() as Vaddr;
        let start_vaddr = self.start_vaddr();

        let offset = vaddr.checked_sub(start_vaddr)?;
        if offset > CHUNK_SIZE {
            return None;
        }

        debug_assert_eq!(offset % ITEM_SIZE, 0);

        Some(offset / ITEM_SIZE)
    }

    pub fn try_dealloc<T>(
        &mut self,
        mut cpu_local: CpuLocal<T, DynamicStorage<T>>,
    ) -> Result<(), CpuLocal<T, DynamicStorage<T>>> {
        let Some(index) = self.get_item_index(&cpu_local) else {
            return Err(cpu_local);
        };

        self.bitmap.set(index, false);
        for cpu in all_cpus() {
            let ptr = cpu_local.storage.get_mut_ptr_on_target(cpu);
            unsafe {
                core::ptr::drop_in_place(ptr);
            }
        }
        let _ = ManuallyDrop::new(cpu_local);
        Ok(())
    }

    pub fn is_full(&self) -> bool {
        self.bitmap.all()
    }

    pub fn is_empty(&self) -> bool {
        self.bitmap.not_any()
    }
}

impl<const ITEM_SIZE: usize> Drop for DynCpuLocalChunk<ITEM_SIZE> {
    fn drop(&mut self) {
        if self.is_empty() {
            unsafe { ManuallyDrop::drop(&mut self.segment) }
        } else {
            panic!("Dropping `DynCpuLocalChunk` while some CPU-local objects are still alive");
        }
    }
}
