use alloc::{collections::btree_map::BTreeMap, sync::Weak, vec::Vec};
use core::ops::Range;

use keyable_arc::KeyableWeak;
use ostd::{
    mm::{PAGE_SIZE, PageFlags, Vaddr, tlb::TlbFlushOp},
    task::disable_preempt,
};

use crate::vm::vmar::{RssType, Vmar, vmar_impls::RssDelta};

pub(crate) struct Rmap {
    entries: BTreeMap<KeyableWeak<Vmar>, Vec<RmapEntry>>,
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct RmapEntry {
    pub vaddr: Vaddr,
    pub offset: usize,
    pub size: usize,
}

impl Rmap {
    pub(in crate::vm) const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, vmar: Weak<Vmar>, entry: RmapEntry) {
        self.entries
            .entry(KeyableWeak::from(vmar))
            .or_default()
            .push(entry)
    }

    pub(crate) fn remove(&mut self, vmar: Weak<Vmar>, vaddr: Vaddr) {
        use alloc::collections::btree_map::Entry;

        let key = KeyableWeak::from(vmar);
        let Entry::Occupied(mut map_entry) = self.entries.entry(key) else {
            panic!("the entry to remove does not exist")
        };

        let entries = map_entry.get_mut();
        let index = entries
            .iter()
            .position(|entry| entry.vaddr == vaddr)
            .expect("the entry to remove does not exist");
        entries.swap_remove(index);

        if entries.is_empty() {
            map_entry.remove();
        }
    }

    pub(crate) fn unmap(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        self.entries.retain(|vmar, entries| {
            let Some(vmar) = vmar.upgrade() else {
                return false;
            };

            let mut rss_delta = RssDelta::new(&vmar);

            for entry in entries {
                let vmo_range =
                    entry.offset.max(offset.start)..(entry.offset + entry.size).min(offset.end);
                if vmo_range.is_empty() {
                    continue;
                }

                let addr_range = (vmo_range.start - entry.offset + entry.vaddr)
                    ..(vmo_range.end - entry.offset + entry.vaddr);

                let preempt_guard = disable_preempt();
                let mut cursor_mut = vmar
                    .vm_space()
                    .cursor_mut(&preempt_guard, &addr_range)
                    .unwrap();
                rss_delta.add(
                    RssType::File,
                    -(cursor_mut.unmap(addr_range.len()) as isize),
                );
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            true
        });
    }

    pub(crate) fn freeze(&mut self, offset: Range<usize>) {
        debug_assert!(offset.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(offset.end.is_multiple_of(PAGE_SIZE));

        self.entries.retain(|vmar, entries| {
            let Some(vmar) = vmar.upgrade() else {
                return false;
            };

            for entry in entries {
                let vmo_range =
                    entry.offset.max(offset.start)..(entry.offset + entry.size).min(offset.end);
                if vmo_range.is_empty() {
                    continue;
                }

                let addr_range = (vmo_range.start - entry.offset + entry.vaddr)
                    ..(vmo_range.end - entry.offset + entry.vaddr);

                let preempt_guard = disable_preempt();
                let mut cursor_mut = vmar
                    .vm_space()
                    .cursor_mut(&preempt_guard, &addr_range)
                    .unwrap();
                loop {
                    let addr = cursor_mut.virt_addr();
                    if addr >= addr_range.end {
                        break;
                    }
                    let len = addr_range.end - addr;
                    if let Some(va) =
                        cursor_mut.protect_next(len, |page_flags, _| *page_flags -= PageFlags::W)
                    {
                        cursor_mut
                            .flusher()
                            .issue_tlb_flush(TlbFlushOp::for_range(va));
                    } else {
                        break;
                    }
                }
                cursor_mut.flusher().dispatch_tlb_flush();
                cursor_mut.flusher().sync_tlb_flush();
            }

            true
        });
    }
}
