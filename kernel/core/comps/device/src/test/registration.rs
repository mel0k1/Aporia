//! Device registration tests.

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::SysObj;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use super::{
    toy::{self, CountingObserver, ToyBlock, ToyDev, ToyDisk, ToyDiskDriver, ToyMatch},
    utils,
};
use crate::{
    bus::BusDevice,
    class::{ClassDevice, ClassObserver},
    common::{BareDevice, DevNum, Error},
};

#[ktest]
fn parentless_class_device_lives_under_virtual() {
    let (class, _) = toy::register_toy_subsystems();
    let dev = ClassDevice::builder(&class, "loop0", ToyDisk { sectors: 1 })
        .devnum(DevNum::block(DeviceId::new(
            MajorId::new(7),
            MinorId::new(0),
        )))
        .build();
    crate::add_device(&dev).unwrap();

    assert_eq!(dev.path(), "/devices/virtual/toyblk/loop0");
    assert_eq!(
        utils::link_target("/class/toyblk/loop0").unwrap(),
        "../../devices/virtual/toyblk/loop0"
    );
    assert_eq!(
        utils::link_target("/devices/virtual/toyblk/loop0/subsystem").unwrap(),
        "../../../../class/toyblk"
    );
    assert!(utils::lookup("/devices/virtual/toyblk/loop0/device").is_none());

    crate::remove_device(&dev).unwrap();
    assert!(utils::lookup("/devices/virtual/toyblk").is_none());
}

#[ktest]
fn class_device_under_class_device_has_no_glue_dir() {
    let (class, _) = toy::register_toy_subsystems();
    let disk = ClassDevice::builder(&class, "sd0", ToyDisk { sectors: 16 }).build();
    crate::add_device(&disk).unwrap();

    let part = ClassDevice::builder(&class, "sd0p1", ToyDisk { sectors: 8 })
        .parent(disk.clone())
        .build();
    crate::add_device(&part).unwrap();
    assert_eq!(part.path(), "/devices/virtual/toyblk/sd0/sd0p1");

    crate::remove_device(&part).unwrap();
    crate::remove_device(&disk).unwrap();
}

#[ktest]
fn unregistered_parent_is_rejected() {
    let (class, _) = toy::register_toy_subsystems();
    let parent = ClassDevice::builder(&class, "orphan-parent", ToyDisk { sectors: 0 }).build();
    let child = ClassDevice::builder(&class, "orphan", ToyDisk { sectors: 0 })
        .parent(parent.clone())
        .build();

    assert!(matches!(
        crate::add_device(&child),
        Err(Error::ParentNotAdded)
    ));
}

#[ktest]
fn duplicate_device_name_is_rejected() {
    let (class, _) = toy::register_toy_subsystems();

    let a = ClassDevice::builder(&class, "twin", ToyDisk { sectors: 0 }).build();
    let b = ClassDevice::builder(&class, "twin", ToyDisk { sectors: 0 }).build();
    crate::add_device(&a).unwrap();
    assert!(matches!(crate::add_device(&b), Err(Error::NameConflict)));
    assert!(utils::lookup("/devices/virtual/toyblk/twin").is_some());
    assert_eq!(
        utils::link_target("/class/toyblk/twin").unwrap(),
        "../../devices/virtual/toyblk/twin"
    );

    crate::remove_device(&a).unwrap();
    assert!(utils::lookup("/class/toyblk/twin").is_none());
}

#[ktest]
fn class_index_conflict_rolls_back_registration() {
    let (class, _) = toy::register_toy_subsystems();

    let p1 = ClassDevice::builder(&class, "p1", ToyDisk { sectors: 0 }).build();
    let p2 = ClassDevice::builder(&class, "p2", ToyDisk { sectors: 0 }).build();
    crate::add_device(&p1).unwrap();
    crate::add_device(&p2).unwrap();
    let c1 = ClassDevice::builder(&class, "same", ToyDisk { sectors: 0 })
        .parent(p1.clone())
        .build();
    let c2 = ClassDevice::builder(&class, "same", ToyDisk { sectors: 0 })
        .parent(p2.clone())
        .build();
    crate::add_device(&c1).unwrap();
    assert!(matches!(crate::add_device(&c2), Err(Error::NameConflict)));
    assert_eq!(
        utils::link_target("/class/toyblk/same").unwrap(),
        "../../devices/virtual/toyblk/p1/same"
    );
    assert!(utils::lookup("/devices/virtual/toyblk/p2/same").is_none());

    crate::remove_device(&c1).unwrap();
    crate::remove_device(&p2).unwrap();
    crate::remove_device(&p1).unwrap();
}

#[ktest]
fn bus_device_registration_and_probe() {
    let (class, bus) = toy::register_toy_subsystems();
    for path in [
        "/bus/toy",
        "/bus/toy/devices",
        "/bus/toy/drivers",
        "/class/toyblk",
    ] {
        assert!(utils::lookup(path).is_some(), "{path} missing");
    }
    assert_eq!(utils::read_attr("/bus/toy", "drivers_autoprobe"), "1\n");

    let driver = bus
        .register_driver(Box::new(ToyDiskDriver {
            match_data: ToyMatch { vendor: 1 },
            class: class.clone(),
            suppress_bind_attrs: false,
        }))
        .unwrap();
    let observer = Arc::new(CountingObserver {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    class
        .register_observer(observer.clone() as Arc<dyn ClassObserver<ToyBlock>>)
        .unwrap();

    let root = BareDevice::new_root("toy0000:00");
    crate::add_device(&root).unwrap();
    let bus_dev = BusDevice::builder(
        &bus,
        "0000:00:01.0",
        ToyDev {
            vendor: 1,
            model: 3,
        },
    )
    .parent(root.clone())
    .dev_type(&toy::DISK_TYPE)
    .build();
    crate::add_device(&bus_dev).unwrap();

    assert_eq!(bus_dev.path(), "/devices/toy0000:00/0000:00:01.0");
    assert_eq!(
        utils::link_target("/devices/toy0000:00/0000:00:01.0/subsystem").unwrap(),
        "../../../bus/toy"
    );
    assert_eq!(
        utils::link_target("/bus/toy/devices/0000:00:01.0").unwrap(),
        "../../../devices/toy0000:00/0000:00:01.0"
    );

    assert!(bus_dev.driver().is_some_and(|d| Arc::ptr_eq(&d, &driver)));
    assert_eq!(
        utils::link_target("/devices/toy0000:00/0000:00:01.0/driver").unwrap(),
        "../../../bus/toy/drivers/toy_disk"
    );
    assert_eq!(
        utils::link_target("/bus/toy/drivers/toy_disk/0000:00:01.0").unwrap(),
        "../../../../devices/toy0000:00/0000:00:01.0"
    );

    let names = utils::attr_names("/devices/toy0000:00/0000:00:01.0");
    for expected in ["uevent", "vendor", "model", "bound_by"] {
        assert!(names.iter().any(|n| n == expected), "{expected} missing");
    }
    assert_eq!(
        utils::read_attr("/devices/toy0000:00/0000:00:01.0", "vendor"),
        "0x0001\n"
    );
    assert_eq!(names.len(), 4);

    let disk_path = "/devices/toy0000:00/0000:00:01.0/toyblk/td3";
    assert!(utils::lookup(disk_path).is_some(), "class device missing");
    assert_eq!(
        utils::link_target(&alloc::format!("{disk_path}/device")).unwrap(),
        "../../../0000:00:01.0"
    );
    assert_eq!(
        utils::link_target(&alloc::format!("{disk_path}/subsystem")).unwrap(),
        "../../../../../class/toyblk"
    );
    assert_eq!(
        utils::link_target("/class/toyblk/td3").unwrap(),
        "../../devices/toy0000:00/0000:00:01.0/toyblk/td3"
    );
    assert_eq!(
        utils::link_target("/dev/block/200:3").unwrap(),
        "../../devices/toy0000:00/0000:00:01.0/toyblk/td3"
    );
    assert_eq!(utils::read_attr(disk_path, "dev"), "200:3\n");
    assert_eq!(utils::read_attr(disk_path, "size"), "8\n");
    assert_eq!(utils::attr_names(disk_path).len(), 3);
    assert_eq!(observer.added.load(Ordering::Relaxed), 1);

    bus.unbind(&bus_dev).unwrap();
    assert!(bus_dev.driver().is_none());
    assert!(utils::lookup("/devices/toy0000:00/0000:00:01.0/driver").is_none());
    assert!(utils::lookup("/bus/toy/drivers/toy_disk/0000:00:01.0").is_none());
    assert!(
        !utils::attr_names("/devices/toy0000:00/0000:00:01.0")
            .iter()
            .any(|n| n == "bound_by")
    );
    assert!(utils::lookup(disk_path).is_none());
    assert!(utils::lookup("/devices/toy0000:00/0000:00:01.0/toyblk").is_none());
    assert!(utils::lookup("/class/toyblk/td3").is_none());
    assert!(utils::lookup("/dev/block/200:3").is_none());
    assert_eq!(observer.removed.load(Ordering::Relaxed), 1);

    let driver_dir = utils::lookup("/bus/toy/drivers/toy_disk")
        .unwrap()
        .cast_to_node()
        .unwrap();
    driver_dir.store_attr("bind", "0000:00:01.0\n").unwrap();
    assert!(bus_dev.driver().is_some());
    assert!(utils::lookup(disk_path).is_some());

    assert!(matches!(
        crate::remove_device(&bus_dev),
        Err(Error::HasChildren)
    ));
    driver_dir.store_attr("unbind", "0000:00:01.0\n").unwrap();
    crate::remove_device(&bus_dev).unwrap();
    assert!(utils::lookup("/devices/toy0000:00/0000:00:01.0").is_none());
    assert!(utils::lookup("/bus/toy/devices/0000:00:01.0").is_none());
    assert!(matches!(
        crate::remove_device(&bus_dev),
        Err(Error::NotAdded)
    ));
    assert!(matches!(
        crate::add_device(&bus_dev),
        Err(Error::AlreadyAdded)
    ));

    crate::remove_device(&root).unwrap();
    bus.unregister_driver(&driver).unwrap();
    class
        .unregister_observer(&(observer as Arc<dyn ClassObserver<ToyBlock>>))
        .unwrap();
}
