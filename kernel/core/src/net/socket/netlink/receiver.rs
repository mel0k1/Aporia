use crate::{events::IoEvents, prelude::*, process::signal::Pollee};

pub(crate) struct MessageReceiver<Message> {
    message_queue: Arc<Mutex<MessageQueue<Message>>>,
    pollee: Pollee,
}

pub(super) struct MessageQueue<Message> {
    messages: VecDeque<Message>,
    total_length: usize,
    error: Option<Error>,
}

impl<Message> MessageQueue<Message> {
    pub(super) fn new_pair(pollee: Pollee) -> (Arc<Mutex<Self>>, MessageReceiver<Message>) {
        let queue = Arc::new(Mutex::new(Self {
            messages: VecDeque::new(),
            total_length: 0,
            error: None,
        }));
        let receiver = MessageReceiver {
            message_queue: queue.clone(),
            pollee,
        };
        (queue, receiver)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub(super) fn has_errors(&self) -> bool {
        self.error.is_some()
    }
}

pub(crate) trait QueueableMessage {
    fn total_len(&self) -> usize;
}

impl<Message: QueueableMessage> MessageQueue<Message> {
    pub(super) fn dequeue_if<F, R>(&mut self, f: F) -> Result<R>
    where
        F: FnOnce(&Message, usize) -> Result<(bool, R)>,
    {
        if let Some(error) = self.error.take() {
            return Err(error);
        }

        let Some(message) = self.messages.front() else {
            return Err(errno!(EAGAIN, "the receive buffer is empty"));
        };

        let length = message.total_len();
        let (should_pop, result) = f(message, length)?;
        if should_pop {
            self.messages.pop_front().unwrap();
            self.total_length -= length;
        }

        Ok(result)
    }

    #[must_use]
    pub(self) fn enqueue(&mut self, message: Message) -> bool {
        let length = message.total_len();

        if NETLINK_DEFAULT_BUF_SIZE - self.total_length < length {
            self.error = Some(errno!(ENOBUFS, "the receive buffer is full",));
            return false;
        }

        self.messages.push_back(message);
        self.total_length += length;

        true
    }
}

impl<Message: QueueableMessage> MessageReceiver<Message> {
    pub(super) fn enqueue_message(&self, message: Message) {
        let is_ok = self.message_queue.lock().enqueue(message);
        if is_ok {
            self.pollee.notify(IoEvents::IN);
        } else {
            self.pollee.notify(IoEvents::ERR);
        }
    }
}

pub(in crate::net::socket) const NETLINK_DEFAULT_BUF_SIZE: usize = 65536;
