use ostd::power::{self, ExitCode};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Priority(u8);

impl Priority {
    pub const LOW: Self = Self(0);

    pub const DEFAULT: Self = Self(128);

    pub const HIGH: Self = Self(192);

    pub const FIRMWARE: Self = Self(224);

    pub const fn new(value: u8) -> Self {
        Self(value)
    }
}

trait RegisteredHandler: inventory::Collect {
    fn callback(&self) -> fn(ExitCode);

    fn priority(&self) -> Priority;
}

#[doc(hidden)]
pub struct RestartHandler {
    callback: fn(ExitCode),
    priority: Priority,
}

impl RestartHandler {
    #[doc(hidden)]
    pub const fn new(callback: fn(ExitCode), priority: Priority) -> Self {
        Self { callback, priority }
    }
}

impl RegisteredHandler for RestartHandler {
    fn callback(&self) -> fn(ExitCode) {
        self.callback
    }

    fn priority(&self) -> Priority {
        self.priority
    }
}

inventory::collect!(RestartHandler);

#[doc(hidden)]
pub struct PoweroffHandler {
    callback: fn(ExitCode),
    priority: Priority,
}

impl PoweroffHandler {
    #[doc(hidden)]
    pub const fn new(callback: fn(ExitCode), priority: Priority) -> Self {
        Self { callback, priority }
    }
}

impl RegisteredHandler for PoweroffHandler {
    fn callback(&self) -> fn(ExitCode) {
        self.callback
    }

    fn priority(&self) -> Priority {
        self.priority
    }
}

inventory::collect!(PoweroffHandler);

#[doc(hidden)]
pub use inventory::submit;

#[macro_export]
macro_rules! register_restart_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::RestartHandler::new($handler, $priority)
        }
    };
}

#[macro_export]
macro_rules! register_poweroff_handler {
    ($handler:path, $priority:expr) => {
        $crate::power::submit! {
            $crate::power::PoweroffHandler::new($handler, $priority)
        }
    };
}

pub(crate) fn init() {
    power::inject_restart_handler(invoke_handlers::<RestartHandler>);
    power::inject_poweroff_handler(invoke_handlers::<PoweroffHandler>);
}

fn invoke_handlers<H: RegisteredHandler>(code: ExitCode) {
    let mut upper_bound = None;
    loop {
        let next_priority = inventory::iter::<H>
            .into_iter()
            .filter(|handler| upper_bound.is_none_or(|bound| handler.priority() < bound))
            .map(RegisteredHandler::priority)
            .max();
        let Some(priority) = next_priority else {
            break;
        };

        for handler in inventory::iter::<H> {
            if handler.priority() == priority {
                (handler.callback())(code);
            }
        }

        upper_bound = Some(priority);
    }
}
