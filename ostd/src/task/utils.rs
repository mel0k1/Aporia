use alloc::fmt;

pub(super) struct ForceSync<T>(T);

impl<T> fmt::Debug for ForceSync<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForceSync").finish_non_exhaustive()
    }
}

unsafe impl<T> Sync for ForceSync<T> {}

impl<T> ForceSync<T> {
    pub(super) fn new(data: T) -> Self {
        Self(data)
    }

    pub(super) unsafe fn get(&self) -> &T {
        &self.0
    }
}
