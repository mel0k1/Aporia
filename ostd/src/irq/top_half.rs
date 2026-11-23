use core::{fmt::Debug, ops::Deref};

use id_alloc::IdAlloc;
use spin::Once;

use crate::{
    Error,
    arch::{
        irq::{HwIrqLine, IRQ_NUM_MAX, IRQ_NUM_MIN, IrqRemapping},
        trap::TrapFrame,
    },
    prelude::*,
    sync::{RwLock, SpinLock, WriteIrqDisabled},
};

pub type IrqCallbackFunction = dyn Fn(&TrapFrame) + Sync + Send + 'static;

#[must_use]
#[derive(Debug)]
pub struct IrqLine {
    inner: Arc<InnerHandle>,
    callbacks: Vec<CallbackHandle>,
}

impl IrqLine {
    pub fn alloc() -> Result<Self> {
        get_or_init_allocator()
            .lock()
            .alloc()
            .map(|id| Self::new(id as u8))
            .ok_or(Error::NotEnoughResources)
    }

    pub fn alloc_specific(irq_num: u8) -> Result<Self> {
        get_or_init_allocator()
            .lock()
            .alloc_specific((irq_num - IRQ_NUM_MIN) as usize)
            .map(|id| Self::new(id as u8))
            .ok_or(Error::NotEnoughResources)
    }

    fn new(index: u8) -> Self {
        let inner = InnerHandle { index };
        inner.remapping.init(index + IRQ_NUM_MIN);

        Self {
            inner: Arc::new(inner),
            callbacks: Vec::new(),
        }
    }

    pub fn num(&self) -> u8 {
        self.inner.index + IRQ_NUM_MIN
    }

    pub fn on_active<F>(&mut self, callback: F)
    where
        F: Fn(&TrapFrame) + Sync + Send + 'static,
    {
        let callback_handle = {
            let callback_box = Box::new(callback);
            let callback_addr = core::ptr::from_ref(&*callback_box).addr();

            let mut callbacks = self.inner.callbacks.write();
            callbacks.push(callback_box);

            CallbackHandle {
                irq_index: self.inner.index,
                callback_addr,
            }
        };

        self.callbacks.push(callback_handle);
    }

    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }

    pub fn remapping_index(&self) -> Option<u16> {
        self.inner.remapping.remapping_index()
    }
}

impl Clone for IrqLine {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            callbacks: Vec::new(),
        }
    }
}

struct Inner {
    callbacks: RwLock<Vec<Box<IrqCallbackFunction>>, WriteIrqDisabled>,
    remapping: IrqRemapping,
}

impl Inner {
    const fn new() -> Self {
        Self {
            callbacks: RwLock::new(Vec::new()),
            remapping: IrqRemapping::new(),
        }
    }
}

const NUMBER_OF_IRQS: usize = (IRQ_NUM_MAX - IRQ_NUM_MIN) as usize + 1;

static INNERS: [Inner; NUMBER_OF_IRQS] = [const { Inner::new() }; NUMBER_OF_IRQS];
static ALLOCATOR: Once<SpinLock<IdAlloc>> = Once::new();

fn get_or_init_allocator() -> &'static SpinLock<IdAlloc> {
    ALLOCATOR.call_once(|| SpinLock::new(IdAlloc::with_capacity(NUMBER_OF_IRQS)))
}

#[must_use]
#[derive(Debug)]
struct InnerHandle {
    index: u8,
}

impl Deref for InnerHandle {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &INNERS[self.index as usize]
    }
}

impl Drop for InnerHandle {
    fn drop(&mut self) {
        ALLOCATOR.get().unwrap().lock().free(self.index as usize);
    }
}

#[must_use]
#[derive(Debug)]
struct CallbackHandle {
    irq_index: u8,
    callback_addr: usize,
}

impl Drop for CallbackHandle {
    fn drop(&mut self) {
        let mut callbacks = INNERS[self.irq_index as usize].callbacks.write();

        let pos = callbacks
            .iter()
            .position(|element| core::ptr::from_ref(&**element).addr() == self.callback_addr);
        let _ = callbacks.swap_remove(pos.unwrap());
    }
}

pub(super) fn process(trap_frame: &TrapFrame, hw_irq_line: &HwIrqLine) {
    let inner = &INNERS[(hw_irq_line.irq_num() - IRQ_NUM_MIN) as usize];
    for callback in &*inner.callbacks.read() {
        callback(trap_frame);
    }
    hw_irq_line.ack();
}

#[cfg(ktest)]
mod test {
    use super::*;

    const IRQ_NUM: u8 = 64;
    const IRQ_INDEX: usize = (IRQ_NUM - IRQ_NUM_MIN) as usize;

    #[ktest]
    fn alloc_and_free_irq() {
        let irq_line = IrqLine::alloc_specific(IRQ_NUM).unwrap();
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        let irq_line_cloned = irq_line.clone();
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        drop(irq_line);
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_err());

        drop(irq_line_cloned);
        assert!(IrqLine::alloc_specific(IRQ_NUM).is_ok());
    }

    #[ktest]
    fn register_and_unregister_callback() {
        let mut irq_line = IrqLine::alloc_specific(IRQ_NUM).unwrap();
        let mut irq_line_cloned = irq_line.clone();

        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 0);

        irq_line.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 1);

        irq_line_cloned.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 2);

        irq_line_cloned.on_active(|_| {});
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 3);

        drop(irq_line);
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 2);

        drop(irq_line_cloned);
        assert_eq!(INNERS[IRQ_INDEX].callbacks.read().len(), 0);
    }
}
