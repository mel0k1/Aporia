use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::fmt::{self, Write};

use aster_systree::{MAX_ATTR_SIZE, SysAttrSet, SysAttrSetBuilder, SysPerms};
use aster_util::printer::VmPrinter;
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    sync::RwMutex,
};

use crate::{
    bus::{Bus, BusDevice, DriverHandle},
    common::{AnyDevice, Error, Result, SysStr},
};

pub type ShowFn<D> = fn(&D, &mut dyn Write) -> Result<()>;

pub type StoreFn<D> = fn(&D, &str) -> Result<()>;

pub struct Attr<D: ?Sized> {
    name: &'static str,
    perms: SysPerms,
    show: Option<ShowFn<D>>,
    store: Option<StoreFn<D>>,
}

impl<D: ?Sized> Attr<D> {
    pub const fn ro(name: &'static str, show: ShowFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
            show: Some(show),
            store: None,
        }
    }

    pub const fn rw(name: &'static str, show: ShowFn<D>, store: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
            show: Some(show),
            store: Some(store),
        }
    }

    pub const fn wo(name: &'static str, store: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::OWNER_W,
            show: None,
            store: Some(store),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn perms(&self) -> SysPerms {
        self.perms
    }
}

impl<D: ?Sized> Clone for Attr<D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: ?Sized> Copy for Attr<D> {}

impl<D: ?Sized> fmt::Debug for Attr<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

#[derive(Clone)]
pub struct TyErasedAttr {
    name: &'static str,
    perms: SysPerms,
    show: Option<TyErasedShowFn>,
    store: Option<TyErasedStoreFn>,
}

impl fmt::Debug for TyErasedAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TyErasedAttr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

impl TyErasedAttr {
    pub(crate) fn from_typed_slice<D: AnyDevice>(attrs: &[Attr<D>]) -> Vec<Self> {
        attrs.iter().map(Self::from_typed).collect()
    }

    pub(crate) fn from_typed<D: AnyDevice>(attr: &Attr<D>) -> Self {
        let show = attr.show.map(|show| -> TyErasedShowFn {
            Arc::new(move |dev: &dyn AnyDevice, w: &mut dyn Write| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                show(dev, w)
            })
        });
        let store = attr.store.map(|store| -> TyErasedStoreFn {
            Arc::new(move |dev: &dyn AnyDevice, s: &str| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                store(dev, s)
            })
        });
        Self {
            name: attr.name,
            perms: attr.perms,
            show,
            store,
        }
    }

    pub(crate) fn from_driver<B: Bus>(driver: &Arc<DriverHandle<B>>) -> Vec<Self> {
        let mut attrs = Vec::new();
        for attr in driver.dev_attrs() {
            let show = attr.show.map(|show| -> TyErasedShowFn {
                let owner = Arc::downgrade(driver);
                Arc::new(move |dev, w| {
                    let dev = dev.as_any().downcast_ref::<BusDevice<B>>().unwrap();
                    let _guard = dev.bind_lock().lock();
                    let driver = dev.driver().ok_or(Error::NotBound)?;
                    if !Weak::ptr_eq(&owner, &Arc::downgrade(&driver)) {
                        return Err(Error::NotBound);
                    }
                    show(dev, w)
                })
            });
            let store = attr.store.map(|store| -> TyErasedStoreFn {
                let owner = Arc::downgrade(driver);
                Arc::new(move |dev, s| {
                    let dev = dev.as_any().downcast_ref::<BusDevice<B>>().unwrap();
                    let _guard = dev.bind_lock().lock();
                    let driver = dev.driver().ok_or(Error::NotBound)?;
                    if !Weak::ptr_eq(&owner, &Arc::downgrade(&driver)) {
                        return Err(Error::NotBound);
                    }
                    store(dev, s)
                })
            });
            attrs.push(Self {
                name: attr.name,
                perms: attr.perms,
                show,
                store,
            });
        }
        attrs
    }

    pub(crate) fn from_dyn(attr: &Attr<dyn AnyDevice>) -> Self {
        Self {
            name: attr.name,
            perms: attr.perms,
            show: attr.show.map(|show| -> TyErasedShowFn { Arc::new(show) }),
            store: attr
                .store
                .map(|store| -> TyErasedStoreFn { Arc::new(store) }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AttrTable {
    inner: RwMutex<TableInner>,
}

impl AttrTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwMutex::new(TableInner {
                set: SysAttrSet::empty().clone(),
                ops: BTreeMap::new(),
            }),
        }
    }

    pub(crate) fn set(&self) -> Arc<SysAttrSet> {
        self.inner.read().set.clone()
    }

    pub(crate) fn add(&self, attrs: Vec<TyErasedAttr>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut seen = BTreeSet::new();
        if attrs
            .iter()
            .any(|attr| inner.ops.contains_key(attr.name) || !seen.insert(attr.name))
        {
            return Err(Error::NameConflict);
        }

        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for attr in &attrs {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        inner.set = Arc::new(builder.build()?);
        for attr in attrs {
            inner.ops.insert(SysStr::from(attr.name), attr);
        }
        Ok(())
    }

    pub(crate) fn remove(&self, names: &[&'static str]) {
        let mut inner = self.inner.write();
        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for name in names {
            inner.ops.remove(*name);
            builder.remove(name);
        }
        let set = builder
            .build()
            .expect("a builder that only removes attributes cannot fail");
        inner.set = Arc::new(set);
    }

    pub(crate) fn show(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        let show = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.show
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let mut printer = VmPrinter::new_skip(writer, offset);
        show(dev, &mut printer)?;
        Ok(printer.bytes_written())
    }

    pub(crate) fn store(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        let store = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.store
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let (text, len) = read_text(reader)?;
        store(dev, &text)?;
        Ok(len)
    }
}

pub(crate) fn read_text(reader: &mut VmReader) -> aster_systree::Result<(String, usize)> {
    let mut buf = vec![0u8; MAX_ATTR_SIZE];
    let mut writer = VmWriter::from(buf.as_mut_slice());
    let len = reader
        .read_fallible(&mut writer)
        .map_err(|_| aster_systree::Error::PageFault)?;
    let text =
        core::str::from_utf8(&buf[..len]).map_err(|_| aster_systree::Error::InvalidOperation)?;
    Ok((String::from(text), len))
}

#[derive(Debug)]
struct TableInner {
    set: Arc<SysAttrSet>,
    ops: BTreeMap<SysStr, TyErasedAttr>,
}

type TyErasedShowFn = Arc<dyn Fn(&dyn AnyDevice, &mut dyn Write) -> Result<()> + Send + Sync>;
type TyErasedStoreFn = Arc<dyn Fn(&dyn AnyDevice, &str) -> Result<()> + Send + Sync>;
