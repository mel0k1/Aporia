use core::{ops::Add, time::Duration};

#[derive(Clone, Copy)]
pub struct SystemTime(Duration);

impl SystemTime {
    pub const UNIX_EPOCH: Self = Self(Duration::ZERO);

    pub fn new(duration: Duration) -> Self {
        Self(duration)
    }

    pub fn since_epoch(self) -> Duration {
        self.0
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;

    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0.saturating_add(rhs))
    }
}
