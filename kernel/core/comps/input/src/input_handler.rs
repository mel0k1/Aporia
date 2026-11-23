use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::{InputDevice, input_dev::InputEvent, unregister_handler_class};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectError {
    IncompatibleDevice,
    DeviceNodeCreationFailed,
    AlreadyConnected,
    InternalError,
}

pub trait InputHandlerClass: Send + Sync + Any + Debug {
    fn name(&self) -> &str;

    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError>;

    fn disconnect(&self, dev: &Arc<dyn InputDevice>);
}

pub trait InputHandler: Send + Sync + Debug {
    fn handle_events(&self, events: &[InputEvent]);
}

#[derive(Clone, Debug)]
pub(crate) struct BoundInputHandler {
    pub(crate) handler: Arc<dyn InputHandler>,
    pub(crate) handler_class: Arc<dyn InputHandlerClass>,
}

#[derive(Debug)]
pub struct RegisteredInputHandlerClass(pub(crate) Arc<dyn InputHandlerClass>);

impl Drop for RegisteredInputHandlerClass {
    fn drop(&mut self) {
        unregister_handler_class(&self.0).unwrap();
    }
}
