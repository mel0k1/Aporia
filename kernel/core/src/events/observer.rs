use super::Events;

pub(crate) trait Observer<E: Events>: Send + Sync {
    fn on_events(&self, events: &E);
}

impl<E: Events> Observer<E> for () {
    fn on_events(&self, _events: &E) {}
}
