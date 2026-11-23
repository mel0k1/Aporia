use std::sync::atomic::Ordering::Relaxed;

use component::init_component;
use first_init::HAS_INIT;

#[init_component]
fn kernel_init() -> Result<(), component::ComponentInitError> {
    Ok(())
}

#[test]
fn test() {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    component::init_all(
        component::InitStage::Bootstrap,
        component::parse_metadata!(),
    )
    .unwrap();
    assert_eq!(HAS_INIT.load(Relaxed), true);
}
