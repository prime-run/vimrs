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

pub struct InputMapper {
    input: Device,
    output: UInputDevice,
    input_state: HashMap<KeyCode, TimeVal>,

    mappings: Vec<Mapping>,

    // NOTE: The most recent candidate for a tap function
    tapping: Option<KeyCode>,

    output_keys: HashSet<KeyCode>,

    /// Keys that should be suppressed (not passed through) until they are physically released.
    /// Used to avoid leaking base keys when a chord remap is broken.
    suppressed_until_released: HashSet<KeyCode>,

    /// Tracks chords that were activated so we know what to suppress if any input is released.
    active_remaps: Vec<ActiveRemap>,

    /// Current active mode. Remaps with a `mode` only apply when this matches.
    active_mode: Option<String>,
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

        // Ensure that any remapped keys are supported by the generated output device
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
                Mapping::ModeSwitch { .. } => {
                    // No outputs to enable
                },
            }
        }

        let output = UInputDevice::create_from_device(&input)
            .context(format!("creating UInputDevice from {}", path.display()))?;

        input
            .grab(GrabMode::Grab)
            .context(format!("grabbing exclusive access on {}", path.display()))?;

        Ok(Self {
            input,
            output,
            input_state: HashMap::new(),
            output_keys: HashSet::new(),
            tapping: None,
            suppressed_until_released: HashSet::new(),
            active_remaps: Vec::new(),
            active_mode: Some("default".to_string()),
            mappings,
        })
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

    /// Compute the effective set of keys that are pressed
    fn compute_keys(&self) -> HashSet<KeyCode> {
        // Start with the input keys
        let mut keys: HashSet<KeyCode> = self
            .input_state
            .keys()
            .cloned()
            .collect();

        // Suppress any base keys flagged until released so we don't leak them
        for s in &self.suppressed_until_released {
            keys.remove(s);
        }

        // First phase is to apply any DualRole mappings as they are likely to
        // be used to produce modifiers when held.
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

    /// Compute the difference between our desired set of keys
    /// and the set of keys that are currently pressed in the
    /// output device.
    /// Release any keys that should not be pressed, and then
    /// press any keys that should be pressed.
    ///
    /// When releasing, release modifiers last so that mappings
    /// that produce eg: CTRL-C don't emit a random C character
    /// when released.
    ///
    /// Similarly, when pressing, emit modifiers first so that
    /// we don't emit C and then CTRL for such a mapping.
    fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()> {
        let desired_keys = self.compute_keys();
        let mut to_release: Vec<KeyCode> = self
            .output_keys
            .difference(&desired_keys)
            .cloned()
            .collect();

        let mut to_press: Vec<KeyCode> = desired_keys
            .difference(&self.output_keys)
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
                        // DualRole has highest precedence; return immediately when mode matches.
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
                        let cand_pri = 1u8; // ModeSwitch wins ties
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
        // the best candidate (if any).
        best_idx
    }

    fn emit_repeat_for_active_remap(&mut self, code: KeyCode, time: &TimeVal) -> Result<bool> {
        let mut dual_idx: Option<usize> = None;
        let mut best_remap_idx: Option<usize> = None;
        let mut best_len: usize = 0;
        for (idx, ar) in self.active_remaps.iter().enumerate() {
            if matches!(ar.kind, ActiveKind::ModeSwitch) {
                continue;
            }
            let mode_ok = match (ar.mode.as_ref(), self.active_mode.as_ref()) {
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
            let len = self.active_remaps[idx]
                .outputs_vec
                .len();
            for i in 0..len {
                let k = self.active_remaps[idx].outputs_vec[i];
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
                let pressed_at = match self.input_state.remove(&code) {
                    None => {
                        self.write_event_and_sync(event)?;
                        return Ok(());
                    },
                    Some(p) => p,
                };

                self.prune_suppressed_keys();

                let mut ended_inputs: Vec<HashSet<KeyCode>> = vec![];
                for ar in &self.active_remaps {
                    if ar.inputs.contains(&code) {
                        ended_inputs.push(ar.inputs.clone());
                    }
                }
                if !ended_inputs.is_empty() {
                    // remove any active remap that referenced the released key
                    self.active_remaps
                        .retain(|ar| !ar.inputs.contains(&code));
                    // suppress any remaining non-modifier inputs that are still held
                    for inputs in ended_inputs {
                        for k in inputs {
                            if k != code && self.input_state.contains_key(&k) && !is_modifier(&k) {
                                self.suppressed_until_released.insert(k);
                            }
                        }
                    }
                }

                self.compute_and_apply_keys(&event.time)?;

                let mut tap_keys: Option<Vec<KeyCode>> = None;
                if let Some(idx) = self.lookup_dual_role_index(code) {
                    if let Mapping::DualRole { tap, .. } = &self.mappings[idx] {
                        tap_keys = Some(tap.clone());
                    }
                }
                if let Some(tap_vec) = tap_keys {
                    // If released quickly enough, becomes a tap press.
                    if let Some(tapping) = self.tapping.take()
                        && tapping == code
                        && timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200)
                    {
                        self.emit_keys(&tap_vec, &event.time, KeyEventType::Press)?;
                        self.emit_keys(&tap_vec, &event.time, KeyEventType::Release)?;
                    }
                }
            },

            KeyEventType::Press => {
                self.input_state
                    .insert(code, event.time);

                self.prune_suppressed_keys();

                match self.lookup_mapping_index(code) {
                    Some(idx) => match &self.mappings[idx] {
                        Mapping::DualRole { .. } => {
                            let (inputs_set, outputs_set, outputs_vec, mode_clone) = {
                                if let Mapping::DualRole { hold, mode, .. } = &self.mappings[idx] {
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
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == inputs_set)
                            {
                                self.active_remaps.push(ActiveRemap {
                                    inputs: inputs_set,
                                    outputs: outputs_set,
                                    outputs_vec,
                                    kind: ActiveKind::DualRole,
                                    mode: mode_clone,
                                });
                            }

                            self.compute_and_apply_keys(&event.time)?;
                            self.tapping.replace(code);
                        },
                        Mapping::Remap { .. } => {
                            let (input_set, output_set, output_vec, mode_clone) = {
                                if let Mapping::Remap { input, output, mode, .. } =
                                    &self.mappings[idx]
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
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == input_set)
                            {
                                self.active_remaps.push(ActiveRemap {
                                    inputs: input_set,
                                    outputs: output_set,
                                    outputs_vec: output_vec,
                                    kind: ActiveKind::Remap,
                                    mode: mode_clone,
                                });
                            }
                            self.compute_and_apply_keys(&event.time)?;
                            self.tapping.replace(code);
                        },
                        Mapping::ModeSwitch { .. } => {
                            let (inputs_vec, inputs_set, mode_new) = {
                                if let Mapping::ModeSwitch { input, mode, .. } = &self.mappings[idx]
                                {
                                    let s: HashSet<KeyCode> = input.clone();
                                    let v: Vec<KeyCode> = s.iter().cloned().collect();
                                    (v, s, mode.clone())
                                } else {
                                    unreachable!()
                                }
                            };

                            for k in &inputs_vec {
                                self.suppressed_until_released
                                    .insert(*k);
                            }

                            self.active_mode = Some(mode_new);

                            if !self
                                .active_remaps
                                .iter()
                                .any(|ar| ar.inputs == inputs_set)
                            {
                                self.active_remaps.push(ActiveRemap {
                                    inputs: inputs_set,
                                    outputs: HashSet::new(),
                                    outputs_vec: Vec::new(),
                                    kind: ActiveKind::ModeSwitch,
                                    mode: None,
                                });
                            }

                            self.compute_and_apply_keys(&event.time)?;
                            self.cancel_pending_tap();
                        },
                    },
                    None => {
                        self.cancel_pending_tap();
                        self.compute_and_apply_keys(&event.time)?;
                    },
                }
            },
            KeyEventType::Repeat => {
                if self.emit_repeat_for_active_remap(code, &event.time)? {
                } else {
                    match self.lookup_mapping_index(code) {
                        Some(idx) => {
                            let mut to_emit: Option<Vec<KeyCode>> = None;
                            match &self.mappings[idx] {
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
                                .suppressed_until_released
                                .contains(&code)
                            {
                                // swallow
                            } else {
                                self.cancel_pending_tap();
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

    fn cancel_pending_tap(&mut self) {
        self.tapping.take();
    }

    fn prune_suppressed_keys(&mut self) {
        self.suppressed_until_released
            .retain(|k| self.input_state.contains_key(k));
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
                    self.output_keys.insert(*key);
                },
                KeyEventType::Release => {
                    self.output_keys.remove(key);
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
        // Neither are modifiers
        Ordering::Equal
    }
}

fn modifiers_last(a: &KeyCode, b: &KeyCode) -> Ordering {
    modifiers_first(a, b).reverse()
}

// fn mode_default() -> Mode {
//     Mode::Normal
// }
