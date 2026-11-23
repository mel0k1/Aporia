mod signal;
mod supervisor;

use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use rustix::process::{self, Pid, Signal};
pub(crate) use signal::SignalGuard;
pub(crate) use supervisor::DaemonSupervisor;

use crate::{config::scheme::DaemonScheme, error::Errno, error_msg};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

struct RunningDaemon<'a> {
    daemon: &'a DaemonScheme,
    child: Child,
    reaped: bool,
}

impl<'a> RunningDaemon<'a> {
    fn new(daemon: &'a DaemonScheme, work_dir: &Path) -> Result<Self, Errno> {
        let mut command = Command::new(&daemon.path);
        command.args(&daemon.args);

        command.current_dir(work_dir);
        command.process_group(0);

        let child = command.spawn().map_err(|err| {
            error_msg!("failed to spawn `{}`: {err}", daemon.path.display());
            Errno::ExecuteCommand
        })?;

        Ok(Self {
            daemon,
            child,
            reaped: false,
        })
    }

    fn executable(&self) -> &Path {
        &self.daemon.path
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }

    fn kill_and_wait(&mut self) {
        self.kill();
        let _ = self.child.wait();
    }

    fn kill(&mut self) {
        if self.reaped {
            return;
        }
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::KILL);
    }

    fn terminate(&self) {
        if self.reaped {
            return;
        }
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::TERM);
    }
}
