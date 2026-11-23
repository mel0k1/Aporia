use alloc::sync::Arc;

use super::{Class, ClassDevice};

pub trait ClassObserver<C: Class>: Send + Sync + 'static {
    fn on_device_added(&self, dev: &Arc<ClassDevice<C>>);

    fn on_device_removed(&self, _dev: &Arc<ClassDevice<C>>) {}
}
