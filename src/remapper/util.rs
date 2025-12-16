use super::types::KeyEventType;
use crate::mapping::{EventCode, KeyCode};
use evdev_rs::{InputEvent, TimeVal};

pub(crate) fn timeval_diff(newer: &TimeVal, older: &TimeVal) -> std::time::Duration {
    const MICROS_PER_SECOND: libc::time_t = 1_000_000;
    let secs = newer.tv_sec - older.tv_sec;
    let usecs = newer.tv_usec - older.tv_usec;

    let (secs, usecs) =
        if usecs < 0 { (secs - 1, usecs + MICROS_PER_SECOND) } else { (secs, usecs) };

    std::time::Duration::from_micros(((secs * MICROS_PER_SECOND) + usecs) as u64)
}

pub(crate) fn make_event(key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> InputEvent {
    InputEvent::new(time, &EventCode::EV_KEY(key), event_type.value())
}

#[inline(always)]
pub(crate) fn is_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KEY_FN
            | KeyCode::KEY_LEFTALT
            | KeyCode::KEY_RIGHTALT
            | KeyCode::KEY_LEFTMETA
            | KeyCode::KEY_RIGHTMETA
            | KeyCode::KEY_LEFTCTRL
            | KeyCode::KEY_RIGHTCTRL
            | KeyCode::KEY_LEFTSHIFT
            | KeyCode::KEY_RIGHTSHIFT
    )
}
