use align_ext::AlignExt;
use ostd::{
    mm::{PageFlags, UFrame, io::util::HasVmReaderWriter, vm_space::VmQueriedItem},
    task::disable_preempt,
};

use super::{Vmar, is_userspace_vaddr};
use crate::{
    prelude::*,
    vm::vmar::{PageFaultInfo, is_userspace_vaddr_range},
};

impl Vmar {
    pub(crate) fn read_alien(
        &self,
        vaddr: Vaddr,
        writer: &mut VmWriter,
    ) -> Result<usize, (Error, usize)> {
        let len = writer.avail();
        let read = |frame: UFrame, skip_offset: usize| {
            let mut reader = frame.reader();
            reader.skip(skip_offset);
            reader.read_fallible(writer)
        };

        self.access_alien(vaddr, len, PageFlags::R, read)
    }

    pub(crate) fn write_alien(
        &self,
        vaddr: Vaddr,
        reader: &mut VmReader,
    ) -> Result<usize, (Error, usize)> {
        let len = reader.remain();
        let write = |frame: UFrame, skip_offset: usize| {
            let mut writer = frame.writer();
            writer.skip(skip_offset);
            writer.write_fallible(reader)
        };

        self.access_alien(vaddr, len, PageFlags::W, write)
    }

    pub(crate) fn fill_zeros_alien(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> Result<usize, (Error, usize)> {
        let mut remain = len;
        let write = |frame: UFrame, skip_offset: usize| {
            let mut writer = frame.writer();
            writer.skip(skip_offset);
            let res = writer.fill_zeros(remain);
            remain -= res;
            Ok(res)
        };

        self.access_alien(vaddr, len, PageFlags::W, write)
    }

    fn access_alien<F>(
        &self,
        vaddr: Vaddr,
        len: usize,
        required_page_flags: PageFlags,
        mut op: F,
    ) -> Result<usize, (Error, usize)>
    where
        F: FnMut(UFrame, usize) -> Result<usize, (ostd::Error, usize)>,
    {
        if len == 0 {
            return Ok(0);
        }

        if !is_userspace_vaddr_range(vaddr, len) {
            return Err((errno!(EINVAL, "the address range is not in userspace"), 0));
        }
        let range = vaddr.align_down(PAGE_SIZE)..(vaddr + len).align_up(PAGE_SIZE);

        let mut current_va = range.start;
        let mut bytes = 0;

        while current_va < range.end {
            let frame = self
                .query_page_with_required_flags(current_va, required_page_flags)
                .map_err(|err| (err, bytes))?;

            let skip_offset = if current_va == range.start {
                vaddr - range.start
            } else {
                0
            };
            match op(frame, skip_offset) {
                Ok(n) => bytes += n,
                Err((err, n)) => return Err((err.into(), bytes + n)),
            }

            current_va += PAGE_SIZE;
        }

        Ok(bytes)
    }

    fn query_page_with_required_flags(
        &self,
        vaddr: Vaddr,
        required_page_flags: PageFlags,
    ) -> Result<UFrame> {
        debug_assert!(is_userspace_vaddr(vaddr) && vaddr.is_multiple_of(PAGE_SIZE));

        let vmspace = self.vm_space();

        loop {
            let preempt_guard = disable_preempt();
            let mut cursor = vmspace.cursor(&preempt_guard, &(vaddr..vaddr + PAGE_SIZE))?;

            match cursor.query()?.1 {
                Some(vm_item) if vm_item.prop().flags.contains(required_page_flags) => {
                    match vm_item {
                        VmQueriedItem::MappedRam { frame, .. } => return Ok((*frame).clone()),
                        VmQueriedItem::MappedIoMem { .. } => {
                            return Err(errno!(
                                EOPNOTSUPP,
                                "accessing alien MMIO memory is not supported currently"
                            ));
                        }
                    }
                }
                Some(_) | None => (),
            }

            drop(cursor);
            drop(preempt_guard);

            let page_fault_info = PageFaultInfo::new(vaddr, required_page_flags.into()).force();
            self.handle_page_fault(&page_fault_info)?;
        }
    }
}
