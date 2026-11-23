use core::cell::Ref;

use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{Fallible, PodAtomic, VmIo},
    task::Task,
};

use crate::{
    prelude::*,
    process::{
        Process, VmarSnapshot,
        posix_thread::{PosixThread, ThreadLocal},
    },
    thread::Thread,
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar, VmarHandle},
};

#[derive(Clone)]
pub struct Context<'a> {
    pub process: Arc<Process>,
    pub(crate) thread_local: &'a ThreadLocal,
    pub posix_thread: &'a PosixThread,
    pub(crate) thread: &'a Thread,
    pub task: &'a Task,
}

impl Context<'_> {
    pub fn user_space(&self) -> CurrentUserSpace<'_> {
        CurrentUserSpace(self.thread_local.vmar().borrow())
    }
}

pub struct CurrentUserSpace<'a>(Ref<'a, Option<VmarHandle>>);

macro_rules! current_userspace {
    () => {
        $crate::context::CurrentUserSpace::new(
            $crate::process::posix_thread::AsThreadLocal::as_thread_local(
                &ostd::task::Task::current().unwrap(),
            )
            .unwrap(),
        )
    };
}

pub(crate) use current_userspace;

impl<'a> CurrentUserSpace<'a> {
    pub(crate) fn new(thread_local: &'a ThreadLocal) -> Self {
        let vmar_ref = thread_local.vmar().borrow();
        Self(vmar_ref)
    }

    pub(crate) fn vmar(&self) -> &Vmar {
        self.0.as_ref().unwrap()
    }

    pub(crate) fn vmar_snapshot(&self) -> VmarSnapshot {
        VmarSnapshot::from(self.0.as_ref().unwrap().clone_weak())
    }

    pub(crate) fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        Ok(self.vmar().vm_space().reader(vaddr, len)?)
    }

    pub(crate) fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        Ok(self.vmar().vm_space().writer(vaddr, len)?)
    }

    pub(crate) fn reader_writer(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> Result<(VmReader<'_, Fallible>, VmWriter<'_, Fallible>)> {
        Ok(self.vmar().vm_space().reader_writer(vaddr, len)?)
    }

    pub(crate) fn atomic_load<T: PodAtomic>(&self, vaddr: Vaddr) -> Result<T> {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        let user_reader = self.reader(vaddr, size_of::<T>())?;
        Ok(user_reader.atomic_load()?)
    }

    pub(crate) fn atomic_fetch_update<T>(&self, vaddr: Vaddr, op: impl Fn(T) -> T) -> Result<T>
    where
        T: PodAtomic + Eq,
    {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        let (reader, writer) = self.reader_writer(vaddr, size_of::<T>())?;

        let mut old_val = reader.atomic_load()?;
        loop {
            match writer.atomic_compare_exchange(&reader, old_val, op(old_val))? {
                (_, true) => return Ok(old_val),
                (cur_val, false) => old_val = cur_val,
            }
        }
    }

    pub(crate) fn read_cstring(&self, vaddr: Vaddr, max_len: usize) -> Result<CString> {
        if max_len > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        let userspace_max_len = VMAR_CAP_ADDR.saturating_sub(vaddr).min(max_len);

        let mut user_reader = self.reader(vaddr, userspace_max_len)?;
        user_reader.read_cstring_until_nul(userspace_max_len)?
            .ok_or_else(|| if userspace_max_len == max_len {
                errno!(
                    ENAMETOOLONG,
                    "the C string does not end before reaching the maximum length"
                )
            } else {
                errno!(
                    EFAULT,
                    "the C string does not end before reaching the maximum userspace virtual address"
                )
            })
    }
}

impl VmIo for CurrentUserSpace<'_> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        let copy_len = writer.avail();

        if copy_len > 0 {
            check_vaddr_lowerbound(offset)?;
        }

        let mut user_reader = self.vmar().vm_space().reader(offset, copy_len)?;
        user_reader.read_fallible(writer).map_err(|err| err.0)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        let copy_len = reader.remain();

        if copy_len > 0 {
            check_vaddr_lowerbound(offset)?;
        }

        let mut user_writer = self.vmar().vm_space().writer(offset, copy_len)?;
        user_writer.write_fallible(reader).map_err(|err| err.0)?;
        Ok(())
    }
}

#[inherit_methods(from = "(**self)")]
impl VmIo for &CurrentUserSpace<'_> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()>;
    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()>;
}

fn check_vaddr_lowerbound(va: Vaddr) -> ostd::Result<()> {
    if va < VMAR_LOWEST_ADDR {
        return Err(ostd::Error::PageFault);
    }
    Ok(())
}

#[macro_export]
macro_rules! current {
    () => {
        $crate::process::Process::current().unwrap()
    };
}

macro_rules! current_thread {
    () => {
        $crate::thread::Thread::current().expect("the current task is not associated with a thread")
    };
}

pub(crate) use current_thread;
