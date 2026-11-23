use core::{
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    MetaSlot, mapping,
    meta::{AnyFrameMeta, get_slot},
    unique::UniqueFrame,
};
use crate::{
    arch::mm::PagingConsts,
    mm::{Paddr, Vaddr},
    panic::abort,
};

pub struct LinkedList<M>
where
    Link<M>: AnyFrameMeta,
{
    front: Option<NonNull<Link<M>>>,
    back: Option<NonNull<Link<M>>>,
    size: usize,
    list_id: u64,
}

unsafe impl<M> Send for LinkedList<M> where Link<M>: AnyFrameMeta {}
unsafe impl<M> Sync for LinkedList<M> where Link<M>: AnyFrameMeta {}

impl<M> Default for LinkedList<M>
where
    Link<M>: AnyFrameMeta,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M> LinkedList<M>
where
    Link<M>: AnyFrameMeta,
{
    pub const fn new() -> Self {
        Self {
            front: None,
            back: None,
            size: 0,
            list_id: 0,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        let is_empty = self.size == 0;
        debug_assert_eq!(is_empty, self.front.is_none());
        debug_assert_eq!(is_empty, self.back.is_none());
        is_empty
    }

    pub fn push_front(&mut self, frame: UniqueFrame<Link<M>>) {
        self.cursor_front_mut().insert_before(frame);
    }

    pub fn pop_front(&mut self) -> Option<UniqueFrame<Link<M>>> {
        self.cursor_front_mut().take_current()
    }

    pub fn push_back(&mut self, frame: UniqueFrame<Link<M>>) {
        self.cursor_at_ghost_mut().insert_before(frame);
    }

    pub fn pop_back(&mut self) -> Option<UniqueFrame<Link<M>>> {
        self.cursor_back_mut().take_current()
    }

    pub fn contains(&mut self, frame: Paddr) -> bool {
        let Ok(slot) = get_slot(frame) else {
            return false;
        };
        slot.in_list.load(Ordering::Relaxed) == self.lazy_get_id()
    }

    pub fn cursor_mut_at(&mut self, frame: Paddr) -> Option<CursorMut<'_, M>> {
        let Ok(slot) = get_slot(frame) else {
            return None;
        };
        let contains = slot.in_list.load(Ordering::Relaxed) == self.lazy_get_id();
        if contains {
            Some(CursorMut {
                list: self,
                current: Some(NonNull::new(slot.as_meta_ptr::<Link<M>>()).unwrap()),
            })
        } else {
            None
        }
    }

    pub fn cursor_front_mut(&mut self) -> CursorMut<'_, M> {
        let current = self.front;
        CursorMut {
            list: self,
            current,
        }
    }

    pub fn cursor_back_mut(&mut self) -> CursorMut<'_, M> {
        let current = self.back;
        CursorMut {
            list: self,
            current,
        }
    }

    fn cursor_at_ghost_mut(&mut self) -> CursorMut<'_, M> {
        CursorMut {
            list: self,
            current: None,
        }
    }

    fn lazy_get_id(&mut self) -> u64 {
        static LIST_ID_ALLOCATOR: AtomicU64 = AtomicU64::new(1);
        const MAX_LIST_ID: u64 = i64::MAX as u64;

        if self.list_id == 0 {
            let id = LIST_ID_ALLOCATOR.fetch_add(1, Ordering::Relaxed);
            if id >= MAX_LIST_ID {
                crate::error!("The frame list ID allocator has exhausted.");
                abort();
            }
            self.list_id = id;
            id
        } else {
            self.list_id
        }
    }
}

pub struct CursorMut<'a, M>
where
    Link<M>: AnyFrameMeta,
{
    list: &'a mut LinkedList<M>,
    current: Option<NonNull<Link<M>>>,
}

impl<M> CursorMut<'_, M>
where
    Link<M>: AnyFrameMeta,
{
    pub fn move_next(&mut self) {
        self.current = match self.current {
            Some(current) => unsafe { current.as_ref().next },
            None => self.list.front,
        };
    }

    pub fn move_prev(&mut self) {
        self.current = match self.current {
            Some(current) => unsafe { current.as_ref().prev },
            None => self.list.back,
        };
    }

    pub fn current_meta(&mut self) -> Option<&mut M> {
        self.current.map(|current| {
            let link_mut = unsafe { &mut *current.as_ptr() };
            &mut link_mut.meta
        })
    }

    pub fn take_current(&mut self) -> Option<UniqueFrame<Link<M>>> {
        let current = self.current?;

        let mut frame = {
            let meta_ptr = current.as_ptr() as *mut MetaSlot;
            let paddr = mapping::meta_to_frame::<PagingConsts>(meta_ptr as Vaddr);
            unsafe { UniqueFrame::<Link<M>>::from_raw(paddr) }
        };

        let next_ptr = frame.meta().next;
        if let Some(prev) = &mut frame.meta_mut().prev {
            let prev_mut = unsafe { prev.as_mut() };

            debug_assert_eq!(prev_mut.next, Some(current));
            prev_mut.next = next_ptr;
        } else {
            self.list.front = next_ptr;
        }
        let prev_ptr = frame.meta().prev;
        if let Some(next) = &mut frame.meta_mut().next {
            let next_mut = unsafe { next.as_mut() };

            debug_assert_eq!(next_mut.prev, Some(current));
            next_mut.prev = prev_ptr;
            self.current = Some(*next);
        } else {
            self.list.back = prev_ptr;
            self.current = None;
        }

        frame.meta_mut().next = None;
        frame.meta_mut().prev = None;

        frame.slot().in_list.store(0, Ordering::Relaxed);

        self.list.size -= 1;

        Some(frame)
    }

    pub fn insert_before(&mut self, mut frame: UniqueFrame<Link<M>>) {
        debug_assert!(frame.meta_mut().next.is_none());
        debug_assert!(frame.meta_mut().prev.is_none());
        debug_assert_eq!(frame.slot().in_list.load(Ordering::Relaxed), 0);

        let frame_ptr = unsafe { NonNull::new_unchecked(frame.slot().as_meta_ptr()) };

        if let Some(current) = &mut self.current {
            let current_mut = unsafe { current.as_mut() };

            if let Some(prev) = &mut current_mut.prev {
                let prev_mut = unsafe { prev.as_mut() };

                debug_assert_eq!(prev_mut.next, Some(*current));
                prev_mut.next = Some(frame_ptr);

                frame.meta_mut().prev = Some(*prev);
                frame.meta_mut().next = Some(*current);
                *prev = frame_ptr;
            } else {
                debug_assert_eq!(self.list.front, Some(*current));
                frame.meta_mut().next = Some(*current);
                current_mut.prev = Some(frame_ptr);
                self.list.front = Some(frame_ptr);
            }
        } else {
            if let Some(back) = &mut self.list.back {
                unsafe {
                    debug_assert!(back.as_mut().next.is_none());
                    back.as_mut().next = Some(frame_ptr);
                }
                frame.meta_mut().prev = Some(*back);
                self.list.back = Some(frame_ptr);
            } else {
                debug_assert_eq!(self.list.front, None);
                self.list.front = Some(frame_ptr);
                self.list.back = Some(frame_ptr);
            }
        }

        frame
            .slot()
            .in_list
            .store(self.list.lazy_get_id(), Ordering::Relaxed);

        let _ = frame.into_raw();

        self.list.size += 1;
    }

    pub fn as_list(&self) -> &LinkedList<M> {
        &*self.list
    }
}

impl<M> Drop for LinkedList<M>
where
    Link<M>: AnyFrameMeta,
{
    fn drop(&mut self) {
        let mut cursor = self.cursor_front_mut();
        while cursor.take_current().is_some() {}
    }
}

#[derive(Debug)]
pub struct Link<M> {
    next: Option<NonNull<Link<M>>>,
    prev: Option<NonNull<Link<M>>>,
    meta: M,
}

unsafe impl<M: Send> Send for Link<M> {}
unsafe impl<M: Sync> Sync for Link<M> {}

impl<M> Deref for Link<M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.meta
    }
}

impl<M> DerefMut for Link<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.meta
    }
}

impl<M> Link<M> {
    pub const fn new(meta: M) -> Self {
        Self {
            next: None,
            prev: None,
            meta,
        }
    }
}

unsafe impl<M> AnyFrameMeta for Link<M>
where
    M: AnyFrameMeta,
{
    fn on_drop(&mut self, reader: &mut crate::mm::VmReader<crate::mm::Infallible>) {
        self.meta.on_drop(reader);
    }

    fn is_untyped(&self) -> bool {
        self.meta.is_untyped()
    }
}
