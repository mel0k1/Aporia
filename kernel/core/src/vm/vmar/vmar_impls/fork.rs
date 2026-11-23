use ostd::{
    mm::{
        CachePolicy, PageFlags,
        tlb::TlbFlushOp,
        vm_space::{CursorMut, VmQueriedItem},
    },
    task::disable_preempt,
};

use super::{RssDelta, Vmar};
use crate::{prelude::*, process::ProcessVm, vm::vmar::VmarHandle};

impl Vmar {
    pub(crate) fn fork_from(vmar: &Self) -> VmarHandle {
        let heap_guard = vmar.process_vm.heap().lock();

        let new_vmar = VmarHandle::new(ProcessVm::fork_from(&vmar.process_vm, &heap_guard));

        {
            let inner = vmar.inner.read();
            let mut new_inner = new_vmar.inner.write();

            let mut rss_delta = RssDelta::new(&new_vmar);
            let mut remaining = inner.vm_mappings.len();

            for vm_mapping in inner.vm_mappings.iter() {
                remaining -= 1;

                let base = vm_mapping.map_to_addr();
                let range = base..vm_mapping.map_end();

                let mut rmap = vm_mapping.lock_rmap();

                let new_mapping = vm_mapping.new_fork();
                new_inner.insert_without_try_merge(&new_vmar, new_mapping, rmap.as_deref_mut());

                let preempt_guard = disable_preempt();
                let new_vmspace = new_vmar.vm_space();
                let mut new_cursor = new_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
                let cur_vmspace = vmar.vm_space();
                let mut cur_cursor = cur_vmspace.cursor_mut(&preempt_guard, &range).unwrap();

                cur_cursor.jump(base).unwrap();
                new_cursor.jump(base).unwrap();

                let num_copied =
                    cow_copy_pt(&mut cur_cursor, &mut new_cursor, vm_mapping.map_size());

                if remaining == 0 {
                    cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::for_all());
                    cur_cursor.flusher().dispatch_tlb_flush();
                    cur_cursor.flusher().sync_tlb_flush();
                }

                rss_delta.add(vm_mapping.rss_type(), num_copied as isize);
            }
        }

        new_vmar
    }
}

fn cow_copy_pt(src: &mut CursorMut<'_>, dst: &mut CursorMut<'_>, size: usize) -> usize {
    let start_va = src.virt_addr();
    let end_va = start_va + size;
    let mut remain_size = size;

    let mut num_copied = 0;

    let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| {
        *flags -= PageFlags::W;
    };

    while let Some(mapped_va) = src.find_next(remain_size) {
        let (va, Some(item)) = src.query().unwrap() else {
            panic!("Found mapped page but query failed");
        };
        debug_assert_eq!(mapped_va, va.start);

        match item {
            VmQueriedItem::MappedRam { frame, mut prop } => {
                let frame = (*frame).clone();

                src.protect_next(end_va - mapped_va, op).unwrap();

                dst.jump(mapped_va).unwrap();
                op(&mut prop.flags, &mut prop.cache);
                dst.map(frame, prop);

                num_copied += 1;
            }
            VmQueriedItem::MappedIoMem { paddr, prop } => {
                let (iomem, offset) = src.find_iomem_by_paddr(paddr).unwrap();
                dst.jump(mapped_va).unwrap();
                dst.map_iomem(iomem, prop, PAGE_SIZE, offset);

                let next_va = mapped_va + PAGE_SIZE;
                if next_va == end_va {
                    break;
                }
                src.jump(next_va).unwrap();
            }
        }

        remain_size = end_va - src.virt_addr();
    }

    num_copied
}

#[cfg(ktest)]
mod test {
    use ostd::{
        io::IoMem,
        mm::{FrameAllocOptions, PageProperty, VmSpace},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn cow_copy_pt_basic() {
        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = 0..PAGE_SIZE * 512 * 512;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let paddr = frame.paddr();
        let frame_clone_for_assert = frame.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame.into(), page_property);

        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut child_cursor, cow_range.len());
            assert_eq!(num_copied, 1);
        };

        {
            let child_map_frame_addr = {
                let mut cursor = child_space.cursor(&preempt_guard, &map_range).unwrap();
                let (_, Some(VmQueriedItem::MappedRam { frame, .. })) = cursor.query().unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                frame.paddr()
            };
            let parent_map_frame_addr = {
                let mut cursor = vm_space.cursor(&preempt_guard, &map_range).unwrap();
                let (_, Some(VmQueriedItem::MappedRam { frame, .. })) = cursor.query().unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                frame.paddr()
            };
            assert_eq!(child_map_frame_addr, parent_map_frame_addr);
            assert_eq!(child_map_frame_addr, paddr);
        }

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
            assert_eq!(num_copied, 0);
        }

        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        drop(vm_space);

        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::R
        ));

        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(frame_clone_for_assert.into(), page_property);

        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedRam { frame, prop }))  if va.start == map_range.start && frame.paddr() == paddr && prop.flags == PageFlags::RW
        ));

        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }

    #[ktest]
    fn cow_copy_pt_iomem() {
        const IOMEM_PADDR: usize = 0x100_000_000_000;

        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = map_range.clone();
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Uncacheable);
        let preempt_guard = disable_preempt();

        let iomem = IoMem::acquire(IOMEM_PADDR..IOMEM_PADDR + PAGE_SIZE)
            .expect("Failed to acquire `IoMem` for testing");
        let iomem_clone_for_assert = iomem.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map_iomem(iomem.clone(), page_property, PAGE_SIZE, 0);

        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut child_cursor, cow_range.len());
            assert_eq!(num_copied, 0);
        };

        {
            let child_map_paddr = {
                let (_, Some(VmQueriedItem::MappedIoMem { paddr, .. })) = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                paddr
            };
            let parent_map_paddr = {
                let (_, Some(VmQueriedItem::MappedIoMem { paddr, .. })) = vm_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                paddr
            };
            assert_eq!(child_map_paddr, parent_map_paddr);
            assert_eq!(child_map_paddr, IOMEM_PADDR);
        }

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let num_copied = cow_copy_pt(&mut parent_cursor, &mut sibling_cursor, cow_range.len());
            assert_eq!(num_copied, 0);
        }

        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        drop(vm_space);

        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map_iomem(iomem_clone_for_assert, page_property, PAGE_SIZE, 0);

        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmQueriedItem::MappedIoMem { paddr, prop }))  if va.start == map_range.start && paddr == IOMEM_PADDR && prop.flags == PageFlags::RW
        ));

        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }
}
