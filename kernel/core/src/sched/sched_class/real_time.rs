use alloc::{collections::VecDeque, sync::Arc};
use core::{
    array,
    num::NonZero,
    sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed},
};

use aster_util::ranged_integer::RangedU8;
use bitvec::{BitArr, bitarr};
use ostd::{
    cpu::CpuId,
    task::{
        Task,
        scheduler::{EnqueueFlags, UpdateFlags},
    },
};

use super::{CurrentRuntime, SchedAttr, SchedClassRq, time::base_slice_clocks};
use crate::thread::AsThread;

pub(crate) type RealTimePriority = RangedU8<1, 99>;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RealTimePolicy {
    Fifo,
    RoundRobin {
        base_slice_factor: Option<NonZero<u32>>,
    },
}

impl Default for RealTimePolicy {
    fn default() -> Self {
        Self::RoundRobin {
            base_slice_factor: None,
        }
    }
}

impl RealTimePolicy {
    fn to_time_slice(self) -> u64 {
        match self {
            RealTimePolicy::RoundRobin { base_slice_factor } => {
                base_slice_clocks()
                    * base_slice_factor
                        .map_or(DEFAULT_BASE_SLICE_FACTOR, |factor| u64::from(factor.get()))
            }
            RealTimePolicy::Fifo => 0,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RealTimeAttr {
    prio: AtomicU8,
    time_slice: AtomicU64,
}

const DEFAULT_BASE_SLICE_FACTOR: u64 = 20;

impl RealTimeAttr {
    pub(crate) fn new(prio: u8, policy: RealTimePolicy) -> Self {
        RealTimeAttr {
            prio: prio.into(),
            time_slice: AtomicU64::new(policy.to_time_slice()),
        }
    }

    pub(crate) fn update(&self, prio: u8, policy: RealTimePolicy) {
        self.prio.store(prio, Relaxed);
        self.time_slice.store(policy.to_time_slice(), Relaxed);
    }
}

struct PrioArray {
    map: BitArr![for 100],
    queue: [VecDeque<Arc<Task>>; 100],
}

impl core::fmt::Debug for PrioArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PrioArray")
            .field_with("map", |f| {
                f.debug_list().entries(self.map.iter_ones()).finish()
            })
            .field_with("queue", |f| {
                f.debug_list()
                    .entries(
                        (self.queue.iter().flatten())
                            .map(|task| task.as_thread().unwrap().sched_attr()),
                    )
                    .finish()
            })
            .finish()
    }
}

impl PrioArray {
    fn enqueue(&mut self, thread: Arc<Task>, prio: u8) {
        let queue = &mut self.queue[usize::from(prio)];
        let is_empty = queue.is_empty();
        queue.push_back(thread);
        if is_empty {
            self.map.set(usize::from(prio), true);
        }
    }

    fn pop(&mut self) -> Option<Arc<Task>> {
        let mut iter = self.map.iter_ones();
        let prio = iter.next()? as u8;

        let queue = &mut self.queue[usize::from(prio)];
        let thread = queue.pop_front().unwrap();

        if queue.is_empty() {
            self.map.set(usize::from(prio), false);
        }
        Some(thread)
    }

    fn peek_prio(&self) -> Option<u8> {
        let prio = self.map.iter_ones().next()?;
        Some(prio as u8)
    }
}

#[derive(Debug)]
pub(super) struct RealTimeClassRq {
    #[expect(unused)]
    cpu: CpuId,
    index: bool,
    array: [PrioArray; 2],
    nr_running: usize,
}

impl RealTimeClassRq {
    pub(crate) fn new(cpu: CpuId) -> RealTimeClassRq {
        RealTimeClassRq {
            cpu,
            index: false,
            array: array::from_fn(|_| PrioArray {
                map: bitarr![0; 100],
                queue: array::from_fn(|_| VecDeque::new()),
            }),
            nr_running: 0,
        }
    }

    fn active_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(self.index)]
    }

    fn inactive_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(!self.index)]
    }

    fn swap_arrays(&mut self) {
        self.index = !self.index;
    }
}

impl SchedClassRq for RealTimeClassRq {
    fn enqueue(&mut self, entity: Arc<Task>, _: Option<EnqueueFlags>) {
        let sched_attr = entity.as_thread().unwrap().sched_attr();
        let prio = sched_attr.real_time.prio.load(Relaxed);
        self.inactive_array().enqueue(entity, prio);
        self.nr_running += 1;
    }

    fn len(&self) -> usize {
        self.nr_running
    }

    fn is_empty(&self) -> bool {
        self.nr_running == 0
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        if self.nr_running == 0 {
            return None;
        }

        (self.active_array().pop())
            .or_else(|| {
                self.swap_arrays();
                self.active_array().pop()
            })
            .inspect(|_| self.nr_running -= 1)
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        let attr = &attr.real_time;

        match flags {
            UpdateFlags::Tick | UpdateFlags::Yield => match attr.time_slice.load(Relaxed) {
                0 => {
                    self.active_array().peek_prio().is_some()
                        || self
                            .inactive_array()
                            .peek_prio()
                            .is_some_and(|prio| prio < attr.prio.load(Relaxed))
                }
                ts => ts <= rt.period_delta && !self.is_empty(),
            },
            UpdateFlags::Wait | UpdateFlags::Exit => !self.is_empty(),
        }
    }
}
