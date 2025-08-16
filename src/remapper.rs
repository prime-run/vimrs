use crate::mapping::*;
use anyhow::*;
use evdev_rs::{Device, DeviceWrapper, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
enum KeyEventType {
    Release,
    Press,
    Repeat,
    Unknown(i32),
}

impl KeyEventType {
    fn from_value(value: i32) -> Self {
        match value {
            0 => KeyEventType::Release,
            1 => KeyEventType::Press,
            2 => KeyEventType::Repeat,
            _ => KeyEventType::Unknown(value),
        }
    }

    fn value(&self) -> i32 {
        match self {
            Self::Release => 0,
            Self::Press => 1,
            Self::Repeat => 2,
            Self::Unknown(n) => *n,
        }
    }
}

fn timeval_diff(newer: &TimeVal, older: &TimeVal) -> Duration {
    const MICROS_PER_SECOND: libc::time_t = 1000000;
    let secs = newer.tv_sec - older.tv_sec;
    let usecs = newer.tv_usec - older.tv_usec;

    let (secs, usecs) =
        if usecs < 0 { (secs - 1, usecs + MICROS_PER_SECOND) } else { (secs, usecs) };

    Duration::from_micros(((secs * MICROS_PER_SECOND) + usecs) as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveKind {
    Remap,
    ModeSwitch,
    DualRole,
}

#[derive(Debug, Clone)]
struct ActiveRemap {
    inputs: HashSet<KeyCode>,
    outputs: HashSet<KeyCode>,
    outputs_vec: Vec<KeyCode>,
    kind: ActiveKind,
    mode: Option<String>,
}

struct RemapEngine {
    input_state: HashMap<KeyCode, TimeVal>,
    mappings: Vec<Mapping>,
    tapping: Option<KeyCode>,
    output_keys: HashSet<KeyCode>,
    suppressed_until_released: HashSet<KeyCode>,
    active_remaps: Vec<ActiveRemap>,
    active_mode: Option<String>,
}

impl RemapEngine {
    fn new(mappings: Vec<Mapping>) -> Self {
        Self {
            input_state: HashMap::new(),
            output_keys: HashSet::new(),
            tapping: None,
            suppressed_until_released: HashSet::new(),
            active_remaps: Vec::new(),
            active_mode: Some("default".to_string()),
            mappings,
        }
    }

    fn compute_keys(&self) -> HashSet<KeyCode> {
        let mut keys: HashSet<KeyCode> = self
            .input_state
            .keys()
            .cloned()
            .collect();
        for s in &self.suppressed_until_released {
            keys.remove(s);
        }

        for map in &self.mappings {
            if let Mapping::DualRole { input, hold, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && keys.contains(input) {
                    keys.remove(input);
                    for h in hold {
                        keys.insert(*h);
                    }
                }
            }
        }

        for ar in &self.active_remaps {
            if ar.kind == ActiveKind::Remap {
                let mode_ok = match (ar.mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok {
                    for i in &ar.inputs {
                        keys.remove(i);
                    }
                    for o in &ar.outputs {
                        keys.insert(*o);
                    }
                }
            }
        }

        keys
    }

    fn lookup_dual_role_index(&self, code: KeyCode) -> Option<usize> {
        for (idx, map) in self.mappings.iter().enumerate() {
            if let Mapping::DualRole { input, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && *input == code {
                    return Some(idx);
                }
            }
        }
        None
    }

    fn lookup_mapping_index(&self, code: KeyCode) -> Option<usize> {
        let mut best_idx: Option<usize> = None;
        let mut best_len: usize = 0;
        let mut best_pri: u8 = 0;
        for (idx, map) in self.mappings.iter().enumerate() {
            match map {
                Mapping::DualRole { input, mode, .. } => {
                    let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_m), None) => false,
                        (Some(m), Some(active)) => m == active,
                    };
                    if mode_ok && *input == code {
                        return Some(idx);
                    }
                },
                Mapping::Remap { input, mode, .. } => {
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_m), None) => false,
                        (Some(m), Some(active)) => m == active,
                    };
                    if code_matched && all_matched && mode_ok {
                        let cand_len = input.len();
                        let cand_pri = 0u8;
                        if best_idx.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_idx = Some(idx);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
                Mapping::ModeSwitch { input, scope, .. } => {
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    let scope_ok = match (scope.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_s), None) => false,
                        (Some(s), Some(active)) => s == active,
                    };
                    if scope_ok && code_matched && all_matched {
                        let cand_len = input.len();
                        let cand_pri = 1u8;
                        if best_idx.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_idx = Some(idx);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
            }
        }
        best_idx
    }

    fn cancel_pending_tap(&mut self) {
        self.tapping.take();
    }

    fn prune_suppressed_keys(&mut self) {
        self.suppressed_until_released
            .retain(|k| self.input_state.contains_key(k));
    }
}

pub struct InputMapper {
    input: Device,
    output: UInputDevice,
    state: RemapEngine,
}

fn enable_key_code(input: &mut Device, key: KeyCode) -> Result<()> {
    input
        .enable(EventCode::EV_KEY(key))
        .context(format!("enable key {key:?}"))?;
    Ok(())
}

impl InputMapper {
    pub fn create_mapper<P: AsRef<Path>>(path: P, mappings: Vec<Mapping>) -> Result<Self> {
        let path = path.as_ref();
        let f = std::fs::File::open(path).context(format!("opening {}", path.display()))?;
        let mut input = Device::new_from_file(f)
            .with_context(|| format!("failed to create new Device from file {}", path.display()))?;

        input.set_name(&format!("evremap Virtual input for {}", path.display()));

        for map in &mappings {
            match map {
                Mapping::DualRole { tap, hold, .. } => {
                    for t in tap {
                        enable_key_code(&mut input, *t)?;
                    }
                    for h in hold {
                        enable_key_code(&mut input, *h)?;
                    }
                },
                Mapping::Remap { output, .. } => {
                    for o in output {
                        enable_key_code(&mut input, *o)?;
                    }
                },
                Mapping::ModeSwitch { .. } => {},
            }
        }

        let output = UInputDevice::create_from_device(&input)
            .context(format!("creating UInputDevice from {}", path.display()))?;

        input
            .grab(GrabMode::Grab)
            .context(format!("grabbing exclusive access on {}", path.display()))?;

        Ok(Self { input, output, state: RemapEngine::new(mappings) })
    }

    pub fn run_mapper(&mut self) -> Result<()> {
        log::info!("Going into read loop");
        loop {
            let (status, event) = self
                .input
                .next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
            match status {
                evdev_rs::ReadStatus::Success => {
                    if let EventCode::EV_KEY(ref key) = event.event_code {
                        log::trace!("IN {event:?}");
                        self.update_with_event(&event, *key)?;
                    } else {
                        log::trace!("PASSTHRU {event:?}");
                        self.output.write_event(&event)?;
                    }
                },
                evdev_rs::ReadStatus::Sync => bail!("ReadStatus::Sync!"),
            }
        }
    }

    fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()> {
        let desired_keys = self.state.compute_keys();
        let mut to_release: Vec<KeyCode> = self
            .state
            .output_keys
            .difference(&desired_keys)
            .cloned()
            .collect();

        let mut to_press: Vec<KeyCode> = desired_keys
            .difference(&self.state.output_keys)
            .cloned()
            .collect();

        if !to_release.is_empty() {
            to_release.sort_by(modifiers_last);
            self.emit_keys(&to_release, time, KeyEventType::Release)?;
        }
        if !to_press.is_empty() {
            to_press.sort_by(modifiers_first);
            self.emit_keys(&to_press, time, KeyEventType::Press)?;
        }
        Ok(())
    }

    fn emit_repeat_for_active_remap(&mut self, code: KeyCode, time: &TimeVal) -> Result<bool> {
        let mut dual_idx: Option<usize> = None;
        let mut best_remap_idx: Option<usize> = None;
        let mut best_len: usize = 0;
        for (idx, ar) in self
            .state
            .active_remaps
            .iter()
            .enumerate()
        {
            if matches!(ar.kind, ActiveKind::ModeSwitch) {
                continue;
            }
            let mode_ok = match (ar.mode.as_ref(), self.state.active_mode.as_ref()) {
                (None, _) => true,
                (Some(_m), None) => false,
                (Some(m), Some(active)) => m == active,
            };
            if mode_ok && ar.inputs.contains(&code) {
                match ar.kind {
                    ActiveKind::DualRole => {
                        dual_idx = Some(idx);
                        break;
                    },
                    ActiveKind::Remap => {
                        let cand_len = ar.inputs.len();
                        if best_remap_idx.is_none() || cand_len > best_len {
                            best_remap_idx = Some(idx);
                            best_len = cand_len;
                        }
                    },
                    ActiveKind::ModeSwitch => {},
                }
            }
        }
        if let Some(idx) = dual_idx.or(best_remap_idx) {
            let len = self.state.active_remaps[idx]
                .outputs_vec
                .len();
            for i in 0..len {
                let k = self.state.active_remaps[idx].outputs_vec[i];
                let event = make_event(k, time, KeyEventType::Repeat);
                self.write_event(&event)?;
            }
            self.generate_sync_event(time)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn update_with_event(&mut self, event: &InputEvent, code: KeyCode) -> Result<()> {
        let event_type = KeyEventType::from_value(event.value);
        match event_type {
            KeyEventType::Release => {
                let pressed_at = match self.state.input_state.remove(&code) {
                    None => {
                        self.write_event_and_sync(event)?;
                        return Ok(());
                    },
                    Some(p) => p,
                };

                self.state.prune_suppressed_keys();

                let mut ended_inputs: Vec<HashSet<KeyCode>> = vec![];
                for ar in &self.state.active_remaps {
                    if ar.inputs.contains(&code) {
                        ended_inputs.push(ar.inputs.clone());
                    }
                }
                if !ended_inputs.is_empty() {
                    self.state
                        .active_remaps
                        .retain(|ar| !ar.inputs.contains(&code));
                    for inputs in ended_inputs {
                        for k in inputs {
                            if k != code
                                && self.state.input_state.contains_key(&k)
                                && !is_modifier(&k)
                            {
                                self.state
                                    .suppressed_until_released
                                    .insert(k);
                            }
                        }
                    }
                }

                self.compute_and_apply_keys(&event.time)?;

                let mut tap_keys: Option<Vec<KeyCode>> = None;
                if let Some(idx) = self.state.lookup_dual_role_index(code) {
                    if let Mapping::DualRole { tap, .. } = &self.state.mappings[idx] {
                        tap_keys = Some(tap.clone());
                    }
                }
                if let Some(tap_vec) = tap_keys {
                    if let Some(tapping) = self.state.tapping.take()
                        && tapping == code
                        && timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200)
                    {
                        self.emit_keys(&tap_vec, &event.time, KeyEventType::Press)?;
                        self.emit_keys(&tap_vec, &event.time, KeyEventType::Release)?;
                    }
                }
            },

            KeyEventType::Press => {
                self.state
                    .input_state
                    .insert(code, event.time);
                self.state.prune_suppressed_keys();

                match self.state.lookup_mapping_index(code) {
                    Some(idx) => match &self.state.mappings[idx] {
                        Mapping::DualRole { .. } => {
                            let (inputs_set, outputs_set, outputs_vec, mode_clone) = {
                                if let Mapping::DualRole { hold, mode, .. } =
                                    &self.state.mappings[idx]
                                {
                                    let mut s: HashSet<KeyCode> = HashSet::new();
                                    s.insert(code);
                                    let vec = hold.clone();
                                    let set: HashSet<KeyCode> = hold.iter().cloned().collect();
                                    (s, set, vec, mode.clone())
                                } else {
                                    unreachable!()
                                }
                            };

                            if !self
                                .state
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == inputs_set)
                            {
                                self.state
                                    .active_remaps
                                    .push(ActiveRemap {
                                        inputs: inputs_set,
                                        outputs: outputs_set,
                                        outputs_vec,
                                        kind: ActiveKind::DualRole,
                                        mode: mode_clone,
                                    });
                            }

                            self.compute_and_apply_keys(&event.time)?;
                            self.state.tapping.replace(code);
                        },
                        Mapping::Remap { .. } => {
                            let (input_set, output_set, output_vec, mode_clone) = {
                                if let Mapping::Remap { input, output, mode, .. } =
                                    &self.state.mappings[idx]
                                {
                                    (
                                        input.clone(),
                                        output.clone(),
                                        output
                                            .iter()
                                            .cloned()
                                            .collect::<Vec<KeyCode>>(),
                                        mode.clone(),
                                    )
                                } else {
                                    unreachable!()
                                }
                            };

                            if !self
                                .state
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == input_set)
                            {
                                self.state
                                    .active_remaps
                                    .push(ActiveRemap {
                                        inputs: input_set,
                                        outputs: output_set,
                                        outputs_vec: output_vec,
                                        kind: ActiveKind::Remap,
                                        mode: mode_clone,
                                    });
                            }
                            self.compute_and_apply_keys(&event.time)?;
                            self.state.tapping.replace(code);
                        },
                        Mapping::ModeSwitch { .. } => {
                            let (inputs_vec, inputs_set, mode_new) = {
                                if let Mapping::ModeSwitch { input, mode, .. } =
                                    &self.state.mappings[idx]
                                {
                                    let s: HashSet<KeyCode> = input.clone();
                                    let v: Vec<KeyCode> = s.iter().cloned().collect();
                                    (v, s, mode.clone())
                                } else {
                                    unreachable!()
                                }
                            };

                            for k in &inputs_vec {
                                self.state
                                    .suppressed_until_released
                                    .insert(*k);
                            }

                            self.state.active_mode = Some(mode_new);

                            if !self
                                .state
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == inputs_set)
                            {
                                self.state
                                    .active_remaps
                                    .push(ActiveRemap {
                                        inputs: inputs_set,
                                        outputs: HashSet::new(),
                                        outputs_vec: Vec::new(),
                                        kind: ActiveKind::ModeSwitch,
                                        mode: None,
                                    });
                            }

                            self.compute_and_apply_keys(&event.time)?;
                            self.state.cancel_pending_tap();
                        },
                    },
                    None => {
                        self.state.cancel_pending_tap();
                        self.compute_and_apply_keys(&event.time)?;
                    },
                }
            },
            KeyEventType::Repeat => {
                if self.emit_repeat_for_active_remap(code, &event.time)? {
                } else {
                    match self.state.lookup_mapping_index(code) {
                        Some(idx) => {
                            let mut to_emit: Option<Vec<KeyCode>> = None;
                            match &self.state.mappings[idx] {
                                Mapping::DualRole { hold, .. } => {
                                    to_emit = Some(hold.clone());
                                },
                                Mapping::Remap { output, .. } => {
                                    to_emit = Some(output.iter().cloned().collect());
                                },
                                Mapping::ModeSwitch { .. } => {},
                            }
                            if let Some(vec) = to_emit {
                                self.emit_keys(&vec, &event.time, KeyEventType::Repeat)?;
                            }
                        },
                        None => {
                            if self
                                .state
                                .suppressed_until_released
                                .contains(&code)
                            {
                            } else {
                                self.state.cancel_pending_tap();
                                self.write_event_and_sync(event)?;
                            }
                        },
                    }
                }
            },
            KeyEventType::Unknown(_) => {
                self.write_event_and_sync(event)?;
            },
        }

        Ok(())
    }

    fn emit_keys(
        &mut self,
        key: &[KeyCode],
        time: &TimeVal,
        event_type: KeyEventType,
    ) -> Result<()> {
        for k in key {
            let event = make_event(*k, time, event_type);
            self.write_event(&event)?;
        }
        self.generate_sync_event(time)?;
        Ok(())
    }

    fn write_event_and_sync(&mut self, event: &InputEvent) -> Result<()> {
        self.write_event(event)?;
        self.generate_sync_event(&event.time)?;
        Ok(())
    }

    fn write_event(&mut self, event: &InputEvent) -> Result<()> {
        log::trace!("OUT: {event:?}");
        self.output.write_event(event)?;
        if let EventCode::EV_KEY(ref key) = event.event_code {
            let event_type = KeyEventType::from_value(event.value);
            match event_type {
                KeyEventType::Press | KeyEventType::Repeat => {
                    self.state.output_keys.insert(*key);
                },
                KeyEventType::Release => {
                    self.state.output_keys.remove(key);
                },
                _ => {},
            }
        }
        Ok(())
    }

    fn generate_sync_event(&self, time: &TimeVal) -> Result<()> {
        self.output
            .write_event(&InputEvent::new(
                time,
                &EventCode::EV_SYN(evdev_rs::enums::EV_SYN::SYN_REPORT),
                0,
            ))?;
        Ok(())
    }
}

fn make_event(key: KeyCode, time: &TimeVal, event_type: KeyEventType) -> InputEvent {
    InputEvent::new(time, &EventCode::EV_KEY(key), event_type.value())
}

fn is_modifier(key: &KeyCode) -> bool {
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

fn modifiers_first(a: &KeyCode, b: &KeyCode) -> Ordering {
    if is_modifier(a) {
        if is_modifier(b) { Ordering::Equal } else { Ordering::Less }
    } else if is_modifier(b) {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

fn modifiers_last(a: &KeyCode, b: &KeyCode) -> Ordering {
    modifiers_first(a, b).reverse()
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use evdev_rs::enums::EV_KEY::*;
//     use std::collections::HashSet;
//
//     #[test]
//     fn basic_remap() {
//         let mappings = vec![Mapping::Remap {
//             input: [KEY_A].iter().cloned().collect(),
//             output: [KEY_X].iter().cloned().collect(),
//             mode: None,
//         }];
//         let mut s = RemapEngine::new(mappings);
//         s.input_state
//             .insert(KEY_A, TimeVal::new(0, 0));
//         s.active_remaps.push(ActiveRemap {
//             inputs: [KEY_A].iter().cloned().collect(),
//             outputs: [KEY_X].iter().cloned().collect(),
//             outputs_vec: vec![KEY_X],
//             kind: ActiveKind::Remap,
//             mode: None,
//         });
//
//         let keys = s.compute_keys();
//         let mut expected = HashSet::new();
//         expected.insert(KEY_X);
//
//         assert_eq!(keys, expected);
//     }
// }
