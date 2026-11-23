mod console;

use aster_framebuffer::mode::ConsoleMode;
use console::VtModeType;
pub(super) use console::{VtConsole, VtMode};
use ostd::sync::{LocalIrqDisabled, WaitQueue};
use spin::Once;

use crate::{
    device::{
        registry::char,
        tty::{
            Tty,
            vt::{
                MAX_CONSOLES, VtIndex,
                c_types::{CVtState, ReleaseDisplayType},
                driver::VtDriver,
            },
        },
    },
    prelude::*,
};

pub(super) struct VtManager {
    terminals: [Arc<Tty<VtDriver>>; MAX_CONSOLES],
    state: SpinLock<VtManagerState, LocalIrqDisabled>,
    vt_active_wq: WaitQueue,
}

pub(super) struct VtManagerGuard<'a> {
    manager: &'a VtManager,
    state: SpinLockGuard<'a, VtManagerState, LocalIrqDisabled>,
}

#[derive(Clone, Copy, Debug)]
struct VtManagerState {
    active: VtIndex,
    wanted: Option<VtIndex>,
    gens: [u64; MAX_CONSOLES],
}

impl VtManagerState {
    fn new(active: VtIndex) -> Self {
        let mut gens = [0u64; MAX_CONSOLES];
        gens[active.to_zero_based()] = 1;

        Self {
            active,
            wanted: None,
            gens,
        }
    }

    fn base_for_relative_switch(&self) -> VtIndex {
        self.wanted.unwrap_or(self.active)
    }
}

impl VtManager {
    const MIN_DEFAULT_ALLOCATED_VT_INDEX: VtIndex = VtIndex::new(1).unwrap();

    const DEFAULT_ACTIVE_VT_INDEX: VtIndex = VtIndex::new(1).unwrap();

    fn new() -> Result<Self> {
        let terminals = core::array::try_from_fn(|i| {
            let vt_index = VtIndex::new((i + 1) as u8).unwrap();
            let driver = VtDriver::new(vt_index);
            let tty = Tty::new(vt_index.get() as u32, driver);
            char::register(tty.clone())?;
            Ok::<Arc<Tty<VtDriver>>, Error>(tty)
        })?;

        for i in 0..Self::MIN_DEFAULT_ALLOCATED_VT_INDEX.get() {
            terminals[i as usize].driver().vt_console().allocate();
        }

        terminals[Self::DEFAULT_ACTIVE_VT_INDEX.to_zero_based()]
            .driver()
            .vt_console()
            .activate();

        Ok(Self {
            terminals,
            state: SpinLock::new(VtManagerState::new(Self::DEFAULT_ACTIVE_VT_INDEX)),
            vt_active_wq: WaitQueue::new(),
        })
    }

    pub(super) fn lock(&self) -> VtManagerGuard<'_> {
        VtManagerGuard {
            manager: self,
            state: self.state.lock(),
        }
    }

    pub(super) fn wait_for_vt_active(&self, target: VtIndex) -> Result<()> {
        let state_lock = self.state.lock();

        if state_lock.active == target {
            return Ok(());
        }

        let start_gen = state_lock.gens[target.to_zero_based()];

        drop(state_lock);

        self.vt_active_wq.pause_until(|| {
            let state_lock = self.state.lock();
            if state_lock.gens[target.to_zero_based()] != start_gen {
                Some(())
            } else {
                None
            }
        })?;

        Ok(())
    }

    pub(super) fn active_vt(&self) -> &Arc<Tty<VtDriver>> {
        let state_lock = self.state.lock();
        self.vt(state_lock.active)
    }

    fn vt(&self, index: VtIndex) -> &Arc<Tty<VtDriver>> {
        &self.terminals[index.to_zero_based()]
    }
}

impl VtManagerGuard<'_> {
    pub(super) fn get_available_vt_index(&self) -> Option<VtIndex> {
        for i in 0..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if !self.is_in_use(idx) {
                return Some(idx);
            }
        }
        None
    }

    pub(super) fn set_vt_mode(&mut self, index: VtIndex, vt_mode: VtMode) {
        if index == self.state.active {
            self.state.wanted = None;
        }

        self.manager
            .vt(index)
            .driver()
            .vt_console()
            .set_vt_mode_and_process(vt_mode, Some(Arc::downgrade(&current!())));
    }

    pub(super) fn set_console_mode(&self, index: VtIndex, mode: ConsoleMode) {
        self.manager.vt(index).driver().vt_console().set_mode(mode);
    }

    pub(super) fn get_global_vt_state(&self) -> CVtState {
        let active_index = self.state.active;

        let mut state_bits: u16 = 1;
        let max_track = (u16::BITS as usize).saturating_sub(1).min(MAX_CONSOLES);

        for i in 0..max_track {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            if self.is_in_use(idx) {
                state_bits |= 1 << (i + 1);
            }
        }

        CVtState {
            active: active_index.get() as u16,
            signal: 0,
            state: state_bits,
        }
    }

    pub(super) fn switch_vt(&mut self, target: VtIndex) -> Result<()> {
        self.switch_vt_inner(target)
    }

    pub(super) fn handle_release_display(
        &mut self,
        vt_index: VtIndex,
        reldisp_type: ReleaseDisplayType,
    ) -> Result<()> {
        let vt_console = self.manager.vt(vt_index).driver().vt_console();
        let mode_type = vt_console.lock_inner().vt_mode().mode_type();
        if mode_type != VtModeType::Process {
            return Err(errno!(EINVAL, "the VT is not in process mode"));
        }

        if vt_index != self.state.active {
            if reldisp_type == ReleaseDisplayType::AckAcquire {
                return Ok(());
            } else {
                return Err(errno!(EINVAL, "the VT is not active"));
            }
        }

        let Some(wanted) = self.state.wanted else {
            if reldisp_type == ReleaseDisplayType::AckAcquire {
                return Ok(());
            } else {
                return Err(errno!(EINVAL, "no VT switch is pending"));
            };
        };

        if reldisp_type == ReleaseDisplayType::DenyRelease {
            self.state.wanted = None;
            return Ok(());
        }

        self.state.wanted = None;

        self.allocate_vt(wanted);

        self.complete_switch_vt(wanted)
    }

    pub(super) fn allocate_vt(&self, index: VtIndex) {
        self.manager.vt(index).driver().vt_console().allocate();
    }

    pub(super) fn disallocate_all_vts(&self) {
        for i in 1..MAX_CONSOLES {
            let idx = VtIndex::new((i + 1) as u8).unwrap();
            let _ = self.disallocate_vt(idx);
        }
    }

    pub(super) fn disallocate_vt(&self, index: VtIndex) -> Result<()> {
        if self.is_busy(index) {
            return Err(errno!(EBUSY, "the VT is busy"));
        }

        if index.get() != 1
            && self.is_allocated(index)
            && index > VtManager::MIN_DEFAULT_ALLOCATED_VT_INDEX
        {
            self.manager.vt(index).driver().vt_console().disallocate();
        }

        Ok(())
    }

    pub(super) fn dec_console(&mut self) -> Result<()> {
        let cur = self.state.base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.prev_wrap();

            if i == cur {
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i);
            }
        }
    }

    pub(super) fn inc_console(&mut self) -> Result<()> {
        let cur = self.state.base_for_relative_switch();

        let mut i = cur;
        loop {
            i = i.next_wrap();

            if i == cur {
                return Ok(());
            }

            if self.is_allocated(i) {
                return self.switch_vt_inner(i);
            }
        }
    }

    fn is_allocated(&self, index: VtIndex) -> bool {
        self.manager.vt(index).driver().vt_console().is_allocated()
    }

    fn is_in_use(&self, index: VtIndex) -> bool {
        let driver = self.manager.vt(index).driver();
        driver.vt_console().is_allocated() && driver.open_file_counter().has_open_files(self)
    }

    fn is_busy(&self, index: VtIndex) -> bool {
        self.is_in_use(index) || index == self.state.active
    }

    fn switch_vt_inner(&mut self, target: VtIndex) -> Result<()> {
        if target == self.state.active {
            return Ok(());
        }

        let active_index = self.state.active;
        let (active_console_mode, active_mode_type) = {
            let active_vt_console = self.manager.vt(active_index).driver().vt_console();
            (
                active_vt_console.mode(),
                active_vt_console.lock_inner().vt_mode().mode_type(),
            )
        };

        if !self.is_allocated(target)
            || (active_console_mode == ConsoleMode::Graphics
                && active_mode_type == VtModeType::Auto)
        {
            return Err(errno!(EINVAL, "VT switching is rejected"));
        }

        if active_mode_type == VtModeType::Process {
            self.state.wanted = Some(target);
            self.manager
                .vt(active_index)
                .driver()
                .vt_console()
                .send_release_signal();
            return Ok(());
        }

        self.complete_switch_vt(target)
    }

    fn complete_switch_vt(&mut self, target: VtIndex) -> Result<()> {
        let old_active_console = self.manager.vt(self.state.active).driver().vt_console();
        old_active_console.deactivate();

        self.state.active = target;
        self.state.gens[target.to_zero_based()] =
            self.state.gens[target.to_zero_based()].wrapping_add(1);

        let new_active_console = self.manager.vt(target).driver().vt_console();
        new_active_console.activate();

        ostd::info!("Switched to VT{}", target.get());

        if new_active_console.lock_inner().vt_mode().mode_type() == VtModeType::Process {
            new_active_console.send_acquire_signal();
        }

        self.manager.vt_active_wq.wake_all();

        Ok(())
    }
}

pub(super) static VT_MANAGER: Once<VtManager> = Once::new();

pub(super) fn init_in_first_process() -> Result<()> {
    let vtm = VtManager::new()?;
    VT_MANAGER.call_once(|| vtm);
    Ok(())
}

pub(crate) fn active_vt() -> &'static Arc<Tty<VtDriver>> {
    let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
    vtm.active_vt()
}

pub(super) fn default_vt() -> &'static Arc<Tty<VtDriver>> {
    let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
    vtm.vt(VtManager::DEFAULT_ACTIVE_VT_INDEX)
}
