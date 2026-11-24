use super::{constants::*, sig_action::SigAction, sig_num::SigNum};
use crate::{
    prelude::*,
    process::signal::{sig_action::SigActionFlags, signals::Signal},
};

#[derive(Clone, Copy)]
pub(crate) struct SigDispositions {
    map: [SigAction; COUNT_ALL_SIGS],
}

impl Default for SigDispositions {
    fn default() -> Self {
        Self::new()
    }
}

impl SigDispositions {
    pub(crate) fn new() -> Self {
        Self {
            map: [SigAction::default(); COUNT_ALL_SIGS],
        }
    }

    pub(crate) fn get(&self, num: SigNum) -> SigAction {
        let idx = Self::num_to_idx(num);
        self.map[idx]
    }

    /// Returns all signal dispositions in ascending signal-number order.
    ///
    /// The iterator covers standard and real-time signals.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (SigNum, SigAction)> + '_ {
        self.map.iter().enumerate().map(|(idx, sig_action)| {
            let sig_num = MIN_STD_SIG_NUM + idx as u8;

            (SigNum::from_u8(sig_num), *sig_action)
        })
    }

    pub(crate) fn set(&mut self, num: SigNum, sa: SigAction) -> Result<SigAction> {
        check_sigaction(&sa)?;
        let idx = Self::num_to_idx(num);
        Ok(core::mem::replace(&mut self.map[idx], sa))
    }

    pub(crate) fn set_default(&mut self, num: SigNum) {
        let idx = Self::num_to_idx(num);
        self.map[idx] = SigAction::Dfl;
    }

    /// man 7 signal:
    /// When execve, the handled signals are reset to the default; the dispositions of
    /// ignored signals are left unchanged.
    /// This function should be used when execve.
    pub(crate) fn inherit(&mut self) {
        for sigaction in &mut self.map {
            if let SigAction::User { .. } = sigaction {
                *sigaction = SigAction::Dfl;
            }
        }
    }

    fn num_to_idx(num: SigNum) -> usize {
        (num.as_u8() - MIN_STD_SIG_NUM) as usize
    }

    pub(crate) fn will_ignore(&self, signal: &dyn Signal) -> bool {
        let signum = signal.num();
        self.get(signum).will_ignore(signum)
    }
}

fn check_sigaction(sig_action: &SigAction) -> Result<()> {
    let SigAction::User {
        flags,
        restorer_addr,
        ..
    } = sig_action
    else {
        return Ok(());
    };

    if flags.contains(SigActionFlags::SA_RESTORER) && *restorer_addr != 0 {
        return Ok(());
    }

    cfg_select! {
        target_arch = "x86_64" => Err(errno!(EINVAL, "x86-64 should always use SA_RESTORER")),
        any(target_arch = "riscv64", target_arch = "aarch64") => Ok(()),
        _ => {
            warn!("SA_RESTORER fallback mechanism not implemented for this architecture");
            Err(errno!(
                EINVAL,
                "this architecture currently requires SA_RESTORER"
            ))
        }
    }
}
