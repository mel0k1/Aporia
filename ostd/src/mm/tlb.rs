use alloc::vec::Vec;
use core::{mem::MaybeUninit, ops::Range, sync::atomic::Ordering};

use super::{
    PAGE_SIZE, Vaddr,
    frame::{Frame, meta::AnyFrameMeta},
};
use crate::{
    const_assert,
    cpu::{AtomicCpuSet, PinCurrentCpu},
    cpu_local,
    smp::{IpiSender, PendingIpis},
    sync::{LocalIrqDisabled, RcuDrop, SpinLock},
};

pub struct TlbFlusher<'a, G: PinCurrentCpu> {
    target_cpus: &'a AtomicCpuSet,
    pending_ipis: PendingIpis,
    ops_stack: OpsStack,
    ipi_sender: Option<&'static IpiSender>,
    _pin_current: G,
}

impl<'a, G: PinCurrentCpu> TlbFlusher<'a, G> {
    pub fn new(target_cpus: &'a AtomicCpuSet, pin_current_guard: G) -> Self {
        Self {
            target_cpus,
            pending_ipis: PendingIpis::new_empty(),
            ops_stack: OpsStack::new(),
            ipi_sender: crate::smp::IPI_SENDER.get(),
            _pin_current: pin_current_guard,
        }
    }

    pub fn issue_tlb_flush(&mut self, op: TlbFlushOp) {
        self.ops_stack.push(op, None);
    }

    pub fn issue_tlb_flush_with(
        &mut self,
        op: TlbFlushOp,
        drop_after_flush: RcuDrop<Frame<dyn AnyFrameMeta>>,
    ) {
        self.ops_stack.push(op, Some(drop_after_flush));
    }

    pub fn dispatch_tlb_flush(&mut self) {
        let irq_guard = crate::irq::disable_local();

        if self.ops_stack.is_empty() {
            return;
        }

        let mut target_cpus = self.target_cpus.load(Ordering::Release);

        let cur_cpu = irq_guard.current_cpu();
        let mut need_flush_on_self = false;

        if target_cpus.contains(cur_cpu) {
            target_cpus.remove(cur_cpu);
            need_flush_on_self = true;
        }

        if let Some(ipi_sender) = self.ipi_sender {
            for cpu in target_cpus.iter() {
                let mut flush_ops = FLUSH_OPS.get_on_cpu(cpu).lock();
                flush_ops.push_from(&self.ops_stack);
            }

            let pending_ipis = ipi_sender.inter_processor_call(&target_cpus, do_remote_flush);
            self.pending_ipis.extend(&pending_ipis);
        }

        if need_flush_on_self {
            self.ops_stack.flush_all();
        } else {
            self.ops_stack.clear_without_flush();
        }
    }

    pub fn sync_tlb_flush(&mut self) {
        if self.ipi_sender.is_none() {
            return;
        }

        self.pending_ipis.wait();
        self.pending_ipis = PendingIpis::new_empty();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlbFlushOp(Vaddr);

const_assert!(TlbFlushOp::FLUSH_RANGE_NPAGES_MASK | (PAGE_SIZE - 1) == PAGE_SIZE - 1);

impl TlbFlushOp {
    const FLUSH_ALL_VAL: Vaddr = Vaddr::MAX;
    const FLUSH_RANGE_NPAGES_MASK: Vaddr =
        (1 << (usize::BITS - FLUSH_ALL_PAGES_THRESHOLD.leading_zeros())) - 1;

    pub fn perform_on_current(&self) {
        use crate::arch::mm;

        match self.0 {
            Self::FLUSH_ALL_VAL => mm::tlb_flush_all_excluding_global(),
            addr => {
                let start = addr & !Self::FLUSH_RANGE_NPAGES_MASK;
                let num_pages = addr & Self::FLUSH_RANGE_NPAGES_MASK;

                debug_assert!((addr & (PAGE_SIZE - 1)) < FLUSH_ALL_PAGES_THRESHOLD);
                debug_assert!(num_pages != 0);

                if num_pages == 1 {
                    mm::tlb_flush_addr(start);
                } else {
                    mm::tlb_flush_addr_range(&(start..start + num_pages * PAGE_SIZE));
                }
            }
        }
    }

    pub const fn for_all() -> Self {
        TlbFlushOp(Self::FLUSH_ALL_VAL)
    }

    pub const fn for_single(addr: Vaddr) -> Self {
        TlbFlushOp(addr | 1)
    }

    pub const fn for_range(range: Range<Vaddr>) -> Self {
        assert!(
            range.start.is_multiple_of(PAGE_SIZE),
            "range start must be page-aligned"
        );
        assert!(
            range.end.is_multiple_of(PAGE_SIZE),
            "range end must be page-aligned"
        );
        assert!(range.start < range.end, "range must not be empty");
        let num_pages = (range.end - range.start) / PAGE_SIZE;
        if num_pages >= FLUSH_ALL_PAGES_THRESHOLD {
            return TlbFlushOp::for_all();
        }
        TlbFlushOp(range.start | (num_pages as Vaddr))
    }

    fn num_pages(&self) -> u32 {
        if self.0 == Self::FLUSH_ALL_VAL {
            u32::MAX
        } else {
            debug_assert!((self.0 & (PAGE_SIZE - 1)) < FLUSH_ALL_PAGES_THRESHOLD);
            let num_pages = (self.0 & Self::FLUSH_RANGE_NPAGES_MASK) as u32;
            debug_assert!(num_pages != 0);
            num_pages
        }
    }
}

cpu_local! {
    static FLUSH_OPS: SpinLock<OpsStack, LocalIrqDisabled> = SpinLock::new(OpsStack::new());
}

fn do_remote_flush() {
    let current_cpu = crate::cpu::CpuId::current_racy();

    let mut new_op_queue = OpsStack::new();
    {
        let mut op_queue = FLUSH_OPS.get_on_cpu(current_cpu).lock();

        core::mem::swap(&mut *op_queue, &mut new_op_queue);
    }
    new_op_queue.flush_all();
}

const FLUSH_ALL_PAGES_THRESHOLD: usize = 32;

struct OpsStack {
    ops: [MaybeUninit<TlbFlushOp>; FLUSH_ALL_PAGES_THRESHOLD],
    num_ops: u32,
    num_pages_to_flush: u32,
    frame_keeper: Vec<Frame<dyn AnyFrameMeta>>,
}

impl OpsStack {
    const fn new() -> Self {
        Self {
            ops: [const { MaybeUninit::uninit() }; FLUSH_ALL_PAGES_THRESHOLD],
            num_ops: 0,
            num_pages_to_flush: 0,
            frame_keeper: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.num_ops == 0 && self.num_pages_to_flush == 0
    }

    fn need_flush_all(&self) -> bool {
        self.num_pages_to_flush == u32::MAX
    }

    fn push(&mut self, op: TlbFlushOp, drop_after_flush: Option<RcuDrop<Frame<dyn AnyFrameMeta>>>) {
        if let Some(frame) = drop_after_flush {
            self.frame_keeper.reserve(1);
            let (frame, panic_guard) = unsafe { RcuDrop::into_inner(frame) };
            self.frame_keeper.push(frame);
            panic_guard.forget();
        }

        if self.need_flush_all() {
            return;
        }
        let op_num_pages = op.num_pages();
        if op == TlbFlushOp::for_all()
            || self.num_pages_to_flush + op_num_pages >= FLUSH_ALL_PAGES_THRESHOLD as u32
        {
            self.num_pages_to_flush = u32::MAX;
            self.num_ops = 0;
            return;
        }

        self.ops[self.num_ops as usize].write(op);
        self.num_ops += 1;
        self.num_pages_to_flush += op_num_pages;
    }

    fn push_from(&mut self, other: &OpsStack) {
        self.frame_keeper.extend(other.frame_keeper.iter().cloned());

        if self.need_flush_all() {
            return;
        }
        if other.need_flush_all()
            || self.num_pages_to_flush + other.num_pages_to_flush
                >= FLUSH_ALL_PAGES_THRESHOLD as u32
        {
            self.num_pages_to_flush = u32::MAX;
            self.num_ops = 0;
            return;
        }

        for other_op in other.ops_iter() {
            self.ops[self.num_ops as usize].write(other_op.clone());
            self.num_ops += 1;
        }
        self.num_pages_to_flush += other.num_pages_to_flush;
    }

    fn flush_all(&mut self) {
        if self.need_flush_all() {
            crate::arch::mm::tlb_flush_all_excluding_global();
        } else {
            self.ops_iter().for_each(|op| {
                op.perform_on_current();
            });
        }

        self.clear_without_flush();
    }

    fn clear_without_flush(&mut self) {
        self.num_pages_to_flush = 0;
        self.num_ops = 0;
        if !self.frame_keeper.is_empty() {
            let _ = RcuDrop::new(core::mem::take(&mut self.frame_keeper));
        }
    }

    fn ops_iter(&self) -> impl Iterator<Item = &TlbFlushOp> {
        self.ops
            .iter()
            .take(self.num_ops as usize)
            .map(|op| unsafe { op.assume_init_ref() })
    }
}

impl Drop for OpsStack {
    fn drop(&mut self) {
        if !self.frame_keeper.is_empty() {
            let _ = RcuDrop::new(core::mem::take(&mut self.frame_keeper));
        }
    }
}
