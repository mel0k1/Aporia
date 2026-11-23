use core::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::prelude::*;

#[expect(clippy::type_complexity)]
pub(crate) struct LazyDelete<T> {
    obj: T,
    is_deleted: AtomicBool,
    delete_fn: Option<Box<dyn FnOnce(&mut T) + Send + Sync>>,
}

impl<T: Debug> Debug for LazyDelete<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyDelete")
            .field("obj", &self.obj)
            .field("is_deleted", &Self::is_deleted(self))
            .finish()
    }
}

impl<T> LazyDelete<T> {
    pub(crate) fn new<F: FnOnce(&mut T) + Send + Sync + 'static>(obj: T, delete_fn: F) -> Self {
        Self {
            obj,
            is_deleted: AtomicBool::new(false),
            delete_fn: Some(Box::new(delete_fn) as _),
        }
    }

    pub(crate) fn delete(this: &Self) {
        this.is_deleted.store(true, Ordering::Release);
    }

    pub(crate) fn is_deleted(this: &Self) -> bool {
        this.is_deleted.load(Ordering::Acquire)
    }
}

impl<T> Deref for LazyDelete<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.obj
    }
}

impl<T> DerefMut for LazyDelete<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.obj
    }
}

impl<T> Drop for LazyDelete<T> {
    fn drop(&mut self) {
        if Self::is_deleted(self) {
            let delete_fn = self.delete_fn.take().unwrap();
            (delete_fn)(&mut self.obj);
        }
    }
}
