//! Class observer tests.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::prelude::ktest;

use super::toy::{self, CountingObserver, ToyBlock, ToyDisk};
use crate::{
    class::{ClassDevice, ClassObserver},
    common::Error,
};

#[ktest]
fn failed_add_does_not_notify_observers() {
    let (class, _) = toy::register_toy_subsystems();
    let observer = Arc::new(CountingObserver {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    class
        .register_observer(observer.clone() as Arc<dyn ClassObserver<ToyBlock>>)
        .unwrap();
    let before_added = observer.added.load(Ordering::Relaxed);

    let a = ClassDevice::builder(&class, "clash", ToyDisk { sectors: 0 }).build();
    let b = ClassDevice::builder(&class, "clash", ToyDisk { sectors: 0 }).build();
    crate::add_device(&a).unwrap();
    assert!(matches!(crate::add_device(&b), Err(Error::NameConflict)));
    assert_eq!(observer.added.load(Ordering::Relaxed), before_added + 1);
    assert_eq!(observer.removed.load(Ordering::Relaxed), 0);

    crate::remove_device(&a).unwrap();
    assert_eq!(observer.removed.load(Ordering::Relaxed), 1);

    class
        .unregister_observer(&(observer as Arc<dyn ClassObserver<ToyBlock>>))
        .unwrap();
}
