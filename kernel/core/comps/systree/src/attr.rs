use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use id_alloc::IdAlloc;
use spin::Once;

use super::{Error, Result, SysStr};
use crate::SysPerms;

#[derive(Clone, Debug)]
pub struct SysAttr {
    id: u8,
    name: SysStr,
    perms: SysPerms,
}

impl SysAttr {
    pub fn new(id: u8, name: SysStr, perms: SysPerms) -> Self {
        Self { id, name, perms }
    }

    pub fn id(&self) -> u8 {
        self.id
    }

    pub fn name(&self) -> &SysStr {
        &self.name
    }

    pub fn perms(&self) -> SysPerms {
        self.perms
    }
}

#[derive(Clone, Debug, Default)]
pub struct SysAttrSet {
    attrs: BTreeMap<SysStr, SysAttr>,
}

impl SysAttrSet {
    pub const CAPACITY: usize = u8::MAX as usize;

    pub const fn new_empty() -> Self {
        Self {
            attrs: BTreeMap::new(),
        }
    }

    pub fn empty() -> &'static Arc<SysAttrSet> {
        static EMPTY: Once<Arc<SysAttrSet>> = Once::new();
        EMPTY.call_once(|| Arc::new(SysAttrSet::new_empty()))
    }

    pub fn get(&self, name: &str) -> Option<&SysAttr> {
        self.attrs.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SysAttr> {
        self.attrs.values()
    }

    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }

    pub fn contains(&self, attr_name: &str) -> bool {
        self.attrs.contains_key(attr_name)
    }
}

#[derive(Debug)]
pub struct SysAttrSetBuilder {
    attrs: BTreeMap<SysStr, SysAttr>,
    ids: IdAlloc,
    error: Option<Error>,
}

impl SysAttrSetBuilder {
    pub fn new() -> Self {
        Self {
            attrs: BTreeMap::new(),
            ids: IdAlloc::with_capacity(SysAttrSet::CAPACITY),
            error: None,
        }
    }

    pub fn from_set(set: &SysAttrSet) -> Self {
        let mut builder = Self::new();
        for attr in set.iter() {
            builder.ids.alloc_specific(attr.id() as usize).unwrap();
            builder.attrs.insert(attr.name().clone(), attr.clone());
        }
        builder
    }

    pub fn add(&mut self, name: SysStr, perms: SysPerms) -> &mut Self {
        if self.error.is_some() {
            return self;
        }
        if !crate::is_valid_name(&name) {
            self.error = Some(Error::InvalidName);
            return self;
        }
        if self.attrs.contains_key(&name) {
            return self;
        }
        let Some(id) = self.ids.alloc() else {
            self.error = Some(Error::ResourceUnavailable);
            return self;
        };
        self.attrs
            .insert(name.clone(), SysAttr::new(id as u8, name, perms));
        self
    }

    pub fn remove(&mut self, name: &str) -> &mut Self {
        if let Some(attr) = self.attrs.remove(name) {
            self.ids.free(attr.id() as usize);
        }
        self
    }

    pub fn build(self) -> Result<SysAttrSet> {
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(SysAttrSet { attrs: self.attrs })
    }
}

impl Default for SysAttrSetBuilder {
    fn default() -> Self {
        Self::new()
    }
}
