use super::{Frame, Segment, meta::AnyFrameMeta};
use crate::mm::{
    HasPaddr, HasSize, Infallible,
    io::{
        VmReader, VmWriter,
        util::{HasVmReaderWriter, VmReaderWriterIdentity},
    },
    paddr_to_vaddr,
};

pub trait AnyUFrameMeta: AnyFrameMeta {}

pub type UFrame = Frame<dyn AnyUFrameMeta>;

#[macro_export]
macro_rules! impl_untyped_frame_meta_for {
    ($t:ty) => {
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {
            fn is_untyped(&self) -> bool {
                true
            }
        }
        impl $crate::mm::frame::untyped::AnyUFrameMeta for $t {}

        $crate::check_frame_meta_layout!($t);
    };
    ($t:ty, $body:expr) => {
        unsafe impl $crate::mm::frame::meta::AnyFrameMeta for $t {
            fn on_drop(&mut self, reader: &mut $crate::mm::VmReader<$crate::mm::Infallible>) {
                $body
            }

            fn is_untyped(&self) -> bool {
                true
            }
        }
        impl $crate::mm::frame::untyped::AnyUFrameMeta for $t {}

        $crate::check_frame_meta_layout!($t);
    };
}

impl_untyped_frame_meta_for!(());

macro_rules! impl_untyped_for {
    ($t:ident) => {
        impl<UM: AnyUFrameMeta + ?Sized> HasVmReaderWriter for $t<UM> {
            type Types = VmReaderWriterIdentity;

            fn reader(&self) -> VmReader<'_, Infallible> {
                let ptr = paddr_to_vaddr(self.paddr()) as *const u8;
                unsafe { VmReader::from_kernel_space(ptr, self.size()) }
            }

            fn writer(&self) -> VmWriter<'_, Infallible> {
                let ptr = paddr_to_vaddr(self.paddr()) as *mut u8;
                unsafe { VmWriter::from_kernel_space(ptr, self.size()) }
            }
        }
    };
}

impl_untyped_for!(Frame);
impl_untyped_for!(Segment);
