use alloc::{boxed::Box, sync::Arc};

use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use super::{
    toy::{self, ToyBlock, ToyBus, ToyDev, ToyDisk, ToyDiskDriver, ToyMatch},
    utils,
};
use crate::{
    bus::BusDevice,
    class::ClassDevice,
    common::{Attr, DevNum, Error},
};

#[ktest]
fn attribute_ids_survive_bind_and_unbind() {
    let (class, bus) = toy::register_toy_subsystems();

    let dev = BusDevice::builder(
        &bus,
        "ids0",
        ToyDev {
            vendor: 9,
            model: 91,
        },
    )
    .build();
    crate::add_device(&dev).unwrap();
    let path = "/devices/ids0";
    let before = utils::attr_ids(path);
    assert!(!before.is_empty());
    assert!(before.iter().all(|(name, _)| name != "bound_by"));

    let driver = bus
        .register_driver(Box::new(ToyDiskDriver {
            match_data: ToyMatch { vendor: 9 },
            class: class.clone(),
            suppress_bind_attrs: false,
        }))
        .unwrap();
    assert!(dev.driver().is_some());
    let driver_attrs = utils::attr_names("/bus/toy/drivers/toy_disk");
    assert!(driver_attrs.iter().any(|name| name == "bind"));
    assert!(driver_attrs.iter().any(|name| name == "unbind"));

    let bound = utils::attr_ids(path);
    for (name, id) in &before {
        assert_eq!(
            bound.iter().find(|(n, _)| n == name).map(|(_, i)| *i),
            Some(*id),
            "attribute `{}` was renumbered by a bind",
            name
        );
    }
    let bound_by_id = bound
        .iter()
        .find(|(n, _)| n == "bound_by")
        .map(|(_, i)| *i)
        .expect("the driver's attribute is present while it is bound");
    assert!(before.iter().all(|(_, id)| *id != bound_by_id));

    bus.unbind(&dev).unwrap();
    assert_eq!(utils::attr_ids(path), before);

    bus.bind(&dev, &driver).unwrap();
    assert_eq!(
        utils::attr_ids(path)
            .iter()
            .find(|(n, _)| n == "bound_by")
            .map(|(_, i)| *i),
        Some(bound_by_id)
    );

    bus.unbind(&dev).unwrap();
    crate::remove_device(&dev).unwrap();
    bus.unregister_driver(&driver).unwrap();
}

#[ktest]
fn an_attribute_may_not_shadow_a_core_one() {
    const SHADOWING: &[Attr<BusDevice<ToyBus>>] = &[Attr::ro("dev", |_dev, w| {
        writeln!(w, "not the core's device number")?;
        Ok(())
    })];

    let (_, bus) = toy::register_toy_subsystems();
    let dev = BusDevice::builder(
        &bus,
        "shadow0",
        ToyDev {
            vendor: 8,
            model: 1,
        },
    )
    .devnum(DevNum::char(DeviceId::new(
        MajorId::new(200),
        MinorId::new(1),
    )))
    .attrs(SHADOWING)
    .build();

    assert!(matches!(crate::add_device(&dev), Err(Error::NameConflict)));
    assert!(utils::lookup("/devices/shadow0").is_none());
}

#[ktest]
fn invalid_attribute_names_are_rejected() {
    const INVALID_ATTRS: &[[Attr<ClassDevice<ToyBlock>>; 1]] = &[
        [Attr::ro("", |_dev, _w| Ok(()))],
        [Attr::ro(".", |_dev, _w| Ok(()))],
        [Attr::ro("..", |_dev, _w| Ok(()))],
        [Attr::ro("bad/name", |_dev, _w| Ok(()))],
        [Attr::ro("bad\0name", |_dev, _w| Ok(()))],
    ];
    let (class, _) = toy::register_toy_subsystems();

    for attrs in INVALID_ATTRS {
        let dev = ClassDevice::builder(&class, "invalid-attr", ToyDisk { sectors: 8 })
            .attrs(attrs)
            .build();
        assert!(matches!(
            crate::add_device(&dev),
            Err(Error::SysTree(aster_systree::Error::InvalidName))
        ));
        assert!(utils::lookup("/devices/virtual/toyblk/invalid-attr").is_none());
        assert!(utils::lookup("/class/toyblk/invalid-attr").is_none());
    }
}

#[ktest]
fn suppressed_bind_attrs_are_inaccessible() {
    let (class, bus) = toy::register_toy_subsystems();
    let dev = BusDevice::builder(
        &bus,
        "suppressed0",
        ToyDev {
            vendor: 101,
            model: 101,
        },
    )
    .build();
    crate::add_device(&dev).unwrap();
    let driver = bus
        .register_driver(Box::new(ToyDiskDriver {
            match_data: ToyMatch { vendor: 101 },
            class: class.clone(),
            suppress_bind_attrs: true,
        }))
        .unwrap();

    let path = "/bus/toy/drivers/toy_disk";
    let attrs = utils::attr_names(path);
    let directory = utils::lookup(path).unwrap().cast_to_node().unwrap();
    let disk = class.find_device("td101").unwrap();

    for name in ["bind", "unbind"] {
        assert!(!attrs.iter().any(|attr| attr == name));
        assert!(matches!(
            directory.show_attr(name),
            Err(aster_systree::Error::NotFound)
        ));
        assert!(matches!(
            directory.store_attr(name, "suppressed0"),
            Err(aster_systree::Error::NotFound)
        ));
    }
    assert!(Arc::ptr_eq(&dev.driver().unwrap(), &driver));
    assert!(Arc::ptr_eq(&class.find_device("td101").unwrap(), &disk));

    bus.unbind(&dev).unwrap();
    assert!(matches!(
        directory.store_attr("bind", "suppressed0"),
        Err(aster_systree::Error::NotFound)
    ));
    assert!(dev.driver().is_none());
    assert!(class.find_device("td101").is_none());

    crate::remove_device(&dev).unwrap();
    bus.unregister_driver(&driver).unwrap();
}
