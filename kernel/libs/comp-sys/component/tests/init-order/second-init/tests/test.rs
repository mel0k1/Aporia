use std::sync::atomic::Ordering::Relaxed;

use second_init::HAS_INIT;

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
