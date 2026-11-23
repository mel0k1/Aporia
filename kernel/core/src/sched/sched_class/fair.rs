use alloc::{collections::BinaryHeap, sync::Arc};
use core::{
    cmp::{self, Reverse},
    sync::atomic::{AtomicU64, Ordering},
};

use ostd::{
    cpu::{CpuId, num_cpus},
    sync::SpinLock,
    task::{
        Task,
        scheduler::{EnqueueFlags, UpdateFlags},
    },
};

use super::{
    CurrentRuntime, SchedAttr, SchedClassRq,
    time::{base_slice_clocks, min_period_clocks},
};
use crate::{
    sched::nice::{Nice, NiceValue},
    thread::AsThread,
};

const WEIGHT_0: u64 = 1024;

const HAS_PENDING: u64 = 1 << (u64::BITS - 1);

pub(crate) const fn nice_to_weight(nice: Nice) -> u64 {
    const FACTOR_NUMERATOR: u64 = 5;
    const FACTOR_DENOMINATOR: u64 = 4;

    const NICE_TO_WEIGHT: [u64; 40] = const {
        let mut ret = [0; 40];

        let mut index = 0;
        let mut nice = NiceValue::MIN.get();
        while nice <= NiceValue::MAX.get() {
            ret[index] = match nice {
                0 => WEIGHT_0,
                nice @ 1.. => {
                    let numerator = FACTOR_DENOMINATOR.pow(nice as u32);
                    let denominator = FACTOR_NUMERATOR.pow(nice as u32);
                    WEIGHT_0 * numerator / denominator
                }
                nice => {
                    let numerator = FACTOR_NUMERATOR.pow((-nice) as u32);
                    let denominator = FACTOR_DENOMINATOR.pow((-nice) as u32);
                    WEIGHT_0 * numerator / denominator
                }
            };
            assert!(ret[index] & HAS_PENDING == 0);

            index += 1;
            nice += 1;
        }
        ret
    };

    NICE_TO_WEIGHT[(nice.value().get() + 20) as usize]
}

#[derive(Debug)]
pub(crate) struct FairAttr {
    weight: AtomicU64,
    pending_weight: SpinLock<u64>,
    vruntime: AtomicU64,
}

impl FairAttr {
    pub(crate) fn new(nice: Nice) -> Self {
        let weight = nice_to_weight(nice);
        FairAttr {
            weight: weight.into(),
            pending_weight: SpinLock::new(weight),
            vruntime: Default::default(),
        }
    }

    pub(crate) fn update(&self, nice: Nice) {
        let mut pending_weight = self.pending_weight.lock();
        *pending_weight = nice_to_weight(nice);
        self.weight.store(
            self.weight.load(Ordering::Relaxed) | HAS_PENDING,
            Ordering::Relaxed,
        );
    }

    fn update_vruntime(&self, delta: u64, weight: u64) -> u64 {
        let delta = delta * WEIGHT_0 / weight;
        self.vruntime.fetch_add(delta, Ordering::Relaxed) + delta
    }

    fn fetch_weight(&self) -> (u64, u64) {
        let weight = self.weight.load(Ordering::Relaxed);
        if weight & HAS_PENDING == 0 {
            return (weight, weight);
        }

        let new_weight = {
            let pending_weight = self.pending_weight.lock();
            self.weight.store(*pending_weight, Ordering::Relaxed);
            *pending_weight
        };
        let old_weight = weight & !HAS_PENDING;

        (old_weight, new_weight)
    }
}

struct FairQueueItem(Arc<Task>, u64);

impl core::fmt::Debug for FairQueueItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.key())
    }
}

impl FairQueueItem {
    fn key(&self) -> u64 {
        self.1
    }
}

impl PartialEq for FairQueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.key().eq(&other.key())
    }
}

impl Eq for FairQueueItem {}

impl PartialOrd for FairQueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FairQueueItem {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

#[derive(Debug)]
pub(super) struct FairClassRq {
    #[expect(unused)]
    cpu: CpuId,
    entities: BinaryHeap<Reverse<FairQueueItem>>,
    min_vruntime: u64,
    total_weight: u64,
}

impl FairClassRq {
    pub(crate) fn new(cpu: CpuId) -> Self {
        Self {
            cpu,
            entities: BinaryHeap::new(),
            min_vruntime: 0,
            total_weight: 0,
        }
    }

    fn period(&self) -> u64 {
        let base_slice_clks = base_slice_clocks();
        let min_period_clks = min_period_clocks();

        let period_single_cpu =
            (base_slice_clks * (self.entities.len() + 1) as u64).max(min_period_clks);
        period_single_cpu * u64::from((1 + num_cpus()).ilog2())
    }

    fn vtime_slice(&self) -> u64 {
        self.period() / (self.entities.len() + 1) as u64
    }

    fn time_slice(&self, cur_weight: u64) -> u64 {
        self.period() * cur_weight / (self.total_weight + cur_weight)
    }
}

impl SchedClassRq for FairClassRq {
    fn enqueue(&mut self, entity: Arc<Task>, flags: Option<EnqueueFlags>) {
        let fair_attr = &entity.as_thread().unwrap().sched_attr().fair;
        let vruntime = match flags {
            Some(EnqueueFlags::Spawn) => self.min_vruntime + self.vtime_slice(),
            _ => self.min_vruntime,
        };
        let (_old_weight, weight) = fair_attr.fetch_weight();

        let vruntime = fair_attr
            .vruntime
            .fetch_max(vruntime, Ordering::Relaxed)
            .max(vruntime);

        self.total_weight += weight;
        self.entities.push(Reverse(FairQueueItem(entity, vruntime)));
    }

    fn len(&self) -> usize {
        self.entities.len()
    }

    fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        let Reverse(FairQueueItem(entity, _)) = self.entities.pop()?;

        let sched_attr = entity.as_thread().unwrap().sched_attr();
        let (old_weight, _weight) = sched_attr.fair.fetch_weight();
        self.total_weight -= old_weight;

        Some(entity)
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        match flags {
            UpdateFlags::Tick | UpdateFlags::Yield | UpdateFlags::Wait => {
                let (_old_weight, weight) = attr.fair.fetch_weight();
                let vruntime = attr.fair.update_vruntime(rt.delta, weight);
                let leftmost = self.entities.peek();
                self.min_vruntime = match leftmost {
                    Some(Reverse(leftmost)) => vruntime.min(leftmost.key()),
                    None => vruntime,
                };
                if leftmost.is_none() {
                    return false;
                }

                matches!(flags, UpdateFlags::Wait)
                    || rt.period_delta > self.time_slice(weight)
                    || vruntime > self.min_vruntime + self.vtime_slice()
            }
            UpdateFlags::Exit => !self.is_empty(),
        }
    }
}
