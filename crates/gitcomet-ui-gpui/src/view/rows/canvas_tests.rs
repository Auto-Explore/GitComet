use super::canvas::take_once_or_debug;

#[test]
fn take_once_or_debug_consumes_present_value_once() {
    let mut slot = Some(42_u32);

    assert_eq!(take_once_or_debug(&mut slot, "slot should exist"), Some(42));
    assert!(slot.is_none());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn take_once_or_debug_panics_in_debug_when_value_is_missing() {
    let mut slot: Option<u8> = None;
    let _ = take_once_or_debug(&mut slot, "slot should exist");
}

#[cfg(not(debug_assertions))]
#[test]
fn take_once_or_debug_returns_none_when_value_is_missing() {
    let mut slot: Option<u8> = None;
    assert!(take_once_or_debug(&mut slot, "slot should exist").is_none());
}
