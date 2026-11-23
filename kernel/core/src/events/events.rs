pub(crate) trait Events: Copy + Clone + Send + Sync + 'static {}

impl Events for () {}

pub(crate) trait EventsFilter<E: Events>: Send + Sync + 'static {
    fn filter(&self, event: &E) -> bool;
}

impl<E: Events> EventsFilter<E> for () {
    fn filter(&self, _events: &E) -> bool {
        true
    }
}

impl<E: Events, F: EventsFilter<E>> EventsFilter<E> for Option<F> {
    fn filter(&self, events: &E) -> bool {
        self.as_ref().is_none_or(|f| f.filter(events))
    }
}
