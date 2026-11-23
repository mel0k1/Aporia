use alloc::{boxed::Box, rc::Rc, sync::Arc};
use core::ops::Range;

use super::Paddr;
use crate::mm::Daddr;

pub trait HasPaddr {
    fn paddr(&self) -> Paddr;
}

pub trait HasDaddr {
    fn daddr(&self) -> Daddr;
}

pub trait HasSize {
    fn size(&self) -> usize;
}

pub trait HasPaddrRange: HasPaddr + HasSize {
    fn end_paddr(&self) -> Paddr;

    fn paddr_range(&self) -> Range<Paddr>;
}

impl<T: HasPaddr + HasSize> HasPaddrRange for T {
    fn end_paddr(&self) -> Paddr {
        self.paddr() + self.size()
    }

    fn paddr_range(&self) -> Range<Paddr> {
        self.paddr()..self.end_paddr()
    }
}

macro_rules! impl_has_traits_for_ref_type {
    ($t:ty, $([$trait_name:ident, $fn_name:ident]),*) => {
        $(
            impl<T: $trait_name> $trait_name for $t {
                fn $fn_name(&self) -> usize {
                    (**self).$fn_name()
                }
            }
        )*
    };
    ($($t:ty),*) => {
        $(
            impl_has_traits_for_ref_type!($t, [HasPaddr, paddr], [HasDaddr, daddr], [HasSize, size]);
        )*
    };
}

impl_has_traits_for_ref_type!(&T, &mut T, Rc<T>, Arc<T>, Box<T>);

pub trait Split: Sized + HasSize {
    fn split(self, offset: usize) -> (Self, Self);
}
