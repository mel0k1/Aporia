use alloc::sync::Arc;

use ostd::{
    sync::non_null::{ArcRef, NonNullPtr},
    util::Either,
};

use crate::node::XNode;

pub(super) type XEntry<P> = Either<Arc<XNode<P>>, P>;

pub(super) type XEntryRef<'a, P> = Either<ArcRef<'a, XNode<P>>, <P as NonNullPtr>::Ref<'a>>;

pub(super) type NodeEntry<P> = Arc<XNode<P>>;

pub(super) type NodeEntryRef<'a, P> = ArcRef<'a, XNode<P>>;
