#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

macro_rules! __log_prefix {
    () => {
        "input: "
    };
}

pub mod event_type_codes;
mod input_core;
pub mod input_dev;
pub mod input_handler;

use alloc::{sync::Arc, vec::Vec};

use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

use self::input_core::InputCore;
use crate::{
    input_dev::{InputDevice, RegisteredInputDevice},
    input_handler::{InputHandlerClass, RegisteredInputHandlerClass},
};

pub fn register_handler_class(
    handler_class: Arc<dyn InputHandlerClass>,
) -> RegisteredInputHandlerClass {
    let component = COMPONENT.get().unwrap();
    component
        .input_core
        .lock()
        .register_handler_class(handler_class.clone());
    RegisteredInputHandlerClass(handler_class)
}

pub(crate) fn unregister_handler_class(
    handler_class: &Arc<dyn InputHandlerClass>,
) -> Option<Arc<dyn InputHandlerClass>> {
    let component = COMPONENT.get().unwrap();
    component
        .input_core
        .lock()
        .unregister_handler_class(handler_class)
}

pub fn register_device(device: Arc<dyn InputDevice>) -> RegisteredInputDevice {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().register_device(device)
}

pub(crate) fn unregister_device(device: &Arc<dyn InputDevice>) -> Option<Arc<dyn InputDevice>> {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().unregister_device(device)
}

pub fn count_devices() -> usize {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().count_devices()
}

pub fn count_handler_class() -> usize {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().count_handler_class()
}

pub fn all_devices() -> Vec<Arc<dyn InputDevice>> {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().all_devices()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);
    Ok(())
}

#[derive(Debug)]
struct Component {
    input_core: Mutex<InputCore>,
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            input_core: Mutex::new(InputCore::new()),
        })
    }
}
