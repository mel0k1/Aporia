use core::cell::{RefCell, RefMut};

pub(crate) struct Mutex<T>(RefCell<T>);

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Sync> Sync for Mutex<T> {}

type MutexGuard<'a, T> = RefMut<'a, T>;

impl<T> Mutex<T> {
    pub(crate) const fn new(data: T) -> Self {
        Self(RefCell::new(data))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
        self.0.borrow_mut()
    }
}
