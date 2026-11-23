pub trait ScheduleNextPoll: Send + Sync {
    fn schedule_next_poll(&self, ms: Option<u64>);
}
