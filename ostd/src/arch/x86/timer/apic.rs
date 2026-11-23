use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    arch::{
        kernel::apic::{self, Apic, DivideConfig},
        timer::pit::OperatingMode,
        trap::TrapFrame,
        tsc_freq,
    },
    info,
    irq::IrqLine,
    task::disable_preempt,
    timer::TIMER_FREQ,
};

pub(super) fn init_on_bsp() -> IrqLine {
    if is_tsc_deadline_mode_supported() {
        init_deadline_mode_config();
    } else {
        init_periodic_mode_config();
    }

    let timer_irq = IrqLine::alloc().unwrap();
    init_timer(&timer_irq);
    timer_irq
}

pub(super) fn init_on_ap(timer_irq: &IrqLine) {
    init_timer(timer_irq);
}

pub(super) fn timer_callback() {
    use x86::msr::{IA32_TSC_DEADLINE, wrmsr};

    match CONFIG.get().expect("ACPI timer config is not initialized") {
        Config::DeadlineMode { tsc_interval } => {
            let tsc_value = crate::arch::read_tsc();
            let next_tsc_value = tsc_interval + tsc_value;
            unsafe { wrmsr(IA32_TSC_DEADLINE, next_tsc_value) };
        }
        Config::PeriodicMode { .. } => {}
    }
}

fn is_tsc_deadline_mode_supported() -> bool {
    use crate::arch::cpu::extension::{IsaExtensions, has_extensions};

    has_extensions(IsaExtensions::TSC_DEADLINE)
}

fn init_timer(timer_irq: &IrqLine) {
    let preempt_guard = disable_preempt();
    let apic = apic::get_or_init(&preempt_guard as _);

    match CONFIG.get().expect("ACPI timer config is not initialized") {
        Config::DeadlineMode { .. } => {
            init_deadline_mode(apic, timer_irq);
        }
        Config::PeriodicMode { init_count } => {
            init_periodic_mode(apic, timer_irq, *init_count);
        }
    }
}

fn init_deadline_mode(apic: &dyn Apic, timer_irq: &IrqLine) {
    apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 18));

    timer_callback();
}

fn init_periodic_mode(apic: &dyn Apic, timer_irq: &IrqLine, init_count: u64) {
    apic.set_timer_init_count(init_count);
    apic.set_lvt_timer(timer_irq.num() as u64 | (1 << 17));
    apic.set_timer_div_config(DivideConfig::Divide64);
}

static CONFIG: spin::Once<Config> = spin::Once::new();

enum Config {
    DeadlineMode { tsc_interval: u64 },
    PeriodicMode { init_count: u64 },
}

fn init_deadline_mode_config() {
    info!("Enable APIC TSC deadline mode");

    let tsc_interval = tsc_freq() / TIMER_FREQ;
    CONFIG.call_once(|| Config::DeadlineMode { tsc_interval });
}

fn init_periodic_mode_config() {
    info!("Enable APIC periodic mode");

    let mut irq = IrqLine::alloc().unwrap();
    irq.on_active(pit_callback);

    super::pit::init(OperatingMode::RateGenerator);
    let irq = super::pit::enable_interrupt(irq);

    let preempt_guard = disable_preempt();
    let apic = apic::get_or_init(&preempt_guard as _);
    apic.set_timer_div_config(DivideConfig::Divide64);
    apic.set_timer_init_count(0xFFFF_FFFF);

    loop {
        crate::arch::irq::enable_local_and_halt();

        crate::arch::irq::disable_local();

        if CONFIG.is_completed() {
            break;
        }
    }

    drop(irq);

    fn pit_callback(_trap_frame: &TrapFrame) {
        static IN_TIME: AtomicU64 = AtomicU64::new(0);
        static APIC_FIRST_COUNT: AtomicU64 = AtomicU64::new(0);
        const CALLBACK_TIMES: u64 = TIMER_FREQ / 10;

        let preempt_guard = disable_preempt();
        let apic = apic::get_or_init(&preempt_guard as _);

        let apic_current_count = 0xFFFF_FFFF - apic.timer_current_count();

        if IN_TIME.load(Ordering::Relaxed) < CALLBACK_TIMES || CONFIG.is_completed() {
            if IN_TIME.load(Ordering::Relaxed) == 0 {
                APIC_FIRST_COUNT.store(apic_current_count, Ordering::Relaxed);
            }
            IN_TIME.fetch_add(1, Ordering::Relaxed);
            return;
        }

        apic.set_timer_init_count(0);

        let apic_first_count = APIC_FIRST_COUNT.load(Ordering::Relaxed);
        let apic_init_count = (apic_current_count - apic_first_count) / CALLBACK_TIMES;
        info!(
            "APIC timer: first {:#x}, current {:#x}, init {:#x}",
            apic_first_count, apic_current_count, apic_init_count,
        );
        CONFIG.call_once(|| Config::PeriodicMode {
            init_count: apic_init_count,
        });
    }
}
