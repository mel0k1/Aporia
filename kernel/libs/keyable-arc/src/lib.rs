#![cfg_attr(not(test), no_std)]
#![feature(coerce_unsized)]
#![feature(unsize)]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::{Arc, Weak};
use core::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    marker::Unsize,
    ops::{CoerceUnsized, Deref},
};

#[repr(transparent)]
pub struct KeyableArc<T: ?Sized>(Arc<T>);

impl<T> KeyableArc<T> {
    #[inline]
    pub fn new(data: T) -> Self {
        Self(Arc::new(data))
    }
}

impl<T: ?Sized> KeyableArc<T> {
    #[inline]
    pub fn as_ptr(this: &Self) -> *const T {
        Arc::as_ptr(&this.0)
    }

    #[inline]
    pub fn downgrade(this: &Self) -> KeyableWeak<T> {
        Arc::downgrade(&this.0).into()
    }

    #[inline]
    pub fn strong_count(this: &Self) -> usize {
        Arc::strong_count(&this.0)
    }

    #[inline]
    pub fn weak_count(this: &Self) -> usize {
        Arc::weak_count(&this.0)
    }
}

impl<T: ?Sized> Deref for KeyableArc<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized> AsRef<T> for KeyableArc<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized> Borrow<T> for KeyableArc<T> {
    #[inline]
    fn borrow(&self) -> &T {
        self
    }
}

impl<T: ?Sized> From<Arc<T>> for KeyableArc<T> {
    #[inline]
    fn from(arc: Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T: ?Sized> From<KeyableArc<T>> for Arc<T> {
    #[inline]
    fn from(value: KeyableArc<T>) -> Self {
        value.0
    }
}

impl<T: ?Sized> PartialEq for KeyableArc<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl<T: ?Sized> Eq for KeyableArc<T> {}

impl<T: ?Sized> Ord for KeyableArc<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        Arc::as_ptr(&self.0)
            .cast::<()>()
            .cmp(&Arc::as_ptr(&other.0).cast::<()>())
    }
}

impl<T: ?Sized> PartialOrd for KeyableArc<T> {
    fn partial_cmp(&self, other: &KeyableArc<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Hash for KeyableArc<T> {
    fn hash<H: Hasher>(&self, s: &mut H) {
        Arc::as_ptr(&self.0).hash(s)
    }
}

impl<T: ?Sized> Clone for KeyableArc<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for KeyableArc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[repr(transparent)]
pub struct KeyableWeak<T: ?Sized>(Weak<T>);

impl<T> KeyableWeak<T> {
    #[expect(clippy::new_without_default)]
    #[inline]
    pub fn new() -> Self {
        Self(Weak::new())
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.0.as_ptr()
    }
}

impl<T: ?Sized> KeyableWeak<T> {
    #[inline]
    pub fn upgrade(&self) -> Option<KeyableArc<T>> {
        self.0.upgrade().map(|arc| arc.into())
    }

    #[inline]
    pub fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    #[inline]
    pub fn weak_count(&self) -> usize {
        self.0.weak_count()
    }
}

impl<T: ?Sized> PartialEq for KeyableWeak<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.0.as_ptr(), other.0.as_ptr())
    }
}

impl<T: ?Sized> Eq for KeyableWeak<T> {}

impl<T: ?Sized> Ord for KeyableWeak<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .as_ptr()
            .cast::<()>()
            .cmp(&other.0.as_ptr().cast::<()>())
    }
}

impl<T: ?Sized> PartialOrd for KeyableWeak<T> {
    fn partial_cmp(&self, other: &KeyableWeak<T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized> Hash for KeyableWeak<T> {
    fn hash<H: Hasher>(&self, s: &mut H) {
        self.0.as_ptr().hash(s)
    }
}

impl<T: ?Sized> From<Weak<T>> for KeyableWeak<T> {
    #[inline]
    fn from(weak: Weak<T>) -> Self {
        Self(weak)
    }
}

impl<T: ?Sized> From<KeyableWeak<T>> for Weak<T> {
    fn from(value: KeyableWeak<T>) -> Self {
        value.0
    }
}

impl<T: ?Sized> Clone for KeyableWeak<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for KeyableWeak<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(KeyableWeak)")
    }
}

impl<T: ?Sized + Unsize<U>, U: ?Sized> CoerceUnsized<KeyableArc<U>> for KeyableArc<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downgrade_and_upgrade() {
        let arc = KeyableArc::new(1);
        let weak = KeyableArc::downgrade(&arc);
        assert!(arc.clone() == weak.upgrade().unwrap());
        assert!(weak == KeyableArc::downgrade(&arc));
    }

    #[test]
    fn debug_format() {
        println!("{:?}", KeyableArc::new(1u32));
        println!("{:?}", KeyableWeak::<u32>::new());
    }

    #[test]
    fn use_as_key() {
        use std::collections::HashMap;

        let mut map: HashMap<KeyableArc<u32>, u32> = HashMap::new();
        let key = KeyableArc::new(1);
        let val = 1;
        map.insert(key.clone(), val);
        assert!(map.get(&key) == Some(&val));
        assert!(map.remove(&key) == Some(val));
        assert!(map.keys().count() == 0);
    }

    #[test]
    fn as_trait_object() {
        trait DummyTrait {}
        struct DummyStruct;
        impl DummyTrait for DummyStruct {}

        let arc_struct = KeyableArc::new(DummyStruct);
        let arc_dyn0: KeyableArc<dyn DummyTrait> = arc_struct.clone();
        let arc_dyn1: KeyableArc<dyn DummyTrait> = arc_struct.clone();
        assert!(arc_dyn0 == arc_dyn1);
    }
}
