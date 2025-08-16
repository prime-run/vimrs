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

// FIX: ...
#[derive(Debug, Clone)]
struct ActiveRemap {
    inputs: HashSet<KeyCode>,
    outputs: HashSet<KeyCode>,
}

pub struct InputMapper {
    input: Device,
    output: UInputDevice,
    /// If present in this map, the key is down since the instant
    /// of its associated value
    input_state: HashMap<KeyCode, TimeVal>,

    mappings: Vec<Mapping>,

    /// The most recent candidate for a tap function is held here
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

        let mut keys_minus_remapped = keys.clone();
        // ModeSwitch base keys are suppressed on press via `suppressed_until_released` in
        // `update_with_event()`, so we don't need to scan all mappings here.

        // Second pass to apply Remap items
        for map in &self.mappings {
            if let Mapping::Remap { input, output, mode } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && input.is_subset(&keys_minus_remapped) {
                    for i in input {
                        keys.remove(i);
                        if !is_modifier(i) {
                            keys_minus_remapped.remove(i);
                        }
                    }
                    for o in output {
                        keys.insert(*o);
                        // Outputs that apply are not visible as
                        // inputs for later remap rules
                        if !is_modifier(o) {
                            keys_minus_remapped.remove(o);
                        }
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

    fn lookup_dual_role_mapping(&self, code: KeyCode) -> Option<Mapping> {
        for map in &self.mappings {
            if let Mapping::DualRole { input, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && *input == code {
                    // A DualRole mapping has the highest precedence
                    // so we've found our match
                    return Some(map.clone());
                }
            }
        }
        None
    }

    fn lookup_mapping(&self, code: KeyCode) -> Option<Mapping> {
        // Track the best candidate on the fly to avoid allocation + sort.
        let mut best_map: Option<&Mapping> = None;
        let mut best_len: usize = 0;
        let mut best_pri: u8 = 0; // Remap=0, ModeSwitch=1 (wins on tie)

        for map in &self.mappings {
            match map {
                Mapping::DualRole { input, .. } => {
                    if *input == code {
                        // DualRole has highest precedence; return immediately.
                        return Some(map.clone());
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
                        if best_map.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_map = Some(map);
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
                        if best_map.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_map = Some(map);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
            }
        }

        best_map.cloned()
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

                // Drop suppressions for keys that are no longer physically held
                self.prune_suppressed_keys();

                // If this release breaks an active remap, suppress the remaining non-modifier
                // inputs in that chord until they are released, and clear the active remap.
                let mut ended_inputs: Vec<HashSet<KeyCode>> = vec![];
                for ar in &self.active_remaps {
                    if ar.inputs.contains(&code) {
                        ended_inputs.push(ar.inputs.clone());
                    }
                }
                if !ended_inputs.is_empty() {
                    // Remove any active remap that referenced the released key
                    self.active_remaps
                        .retain(|ar| !ar.inputs.contains(&code));
                    // Suppress any remaining non-modifier inputs that are still held
                    for inputs in ended_inputs {
                        for k in inputs {
                            if k != code && self.input_state.contains_key(&k) && !is_modifier(&k) {
                                self.suppressed_until_released.insert(k);
                            }
                        }
                    }
                }

                self.compute_and_apply_keys(&event.time)?;

                if let Some(Mapping::DualRole { tap, .. }) = self.lookup_dual_role_mapping(code) {
                    // If released quickly enough, becomes a tap press.
                    if let Some(tapping) = self.tapping.take()
                        && tapping == code
                        && timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200)
                    {
                        self.emit_keys(&tap, &event.time, KeyEventType::Press)?;
                        self.emit_keys(&tap, &event.time, KeyEventType::Release)?;
                    }
                }
            },

            KeyEventType::Press => {
                self.input_state
                    .insert(code, event.time);

                // Drop suppressions for keys that are no longer physically held
                self.prune_suppressed_keys();

                match self.lookup_mapping(code) {
                    Some(Mapping::DualRole { .. }) => {
                        self.compute_and_apply_keys(&event.time)?;
                        self.tapping.replace(code);
                    },
                    Some(Mapping::Remap { input, output, .. }) => {
                        // Register active remap for this chord if not already present
                        if !self
                            .active_remaps
                            .iter()
                            .any(|ar| ar.inputs == input)
                        {
                            self.active_remaps.push(ActiveRemap {
                                inputs: input.clone(),
                                outputs: output.clone(),
                            });
                        }
                        self.compute_and_apply_keys(&event.time)?;
                        self.tapping.replace(code);
                    },
                    Some(Mapping::ModeSwitch { input, mode, .. }) => {
                        // Consume base keys for this switch so they don't leak through
                        for k in input.iter() {
                            self.suppressed_until_released
                                .insert(*k);
                        }

                        // Persistently switch active mode
                        self.active_mode = Some(mode.clone());

                        // Track for suppression management on release
                        if !self
                            .active_remaps
                            .iter()
                            .any(|ar| ar.inputs == input)
                        {
                            self.active_remaps.push(ActiveRemap {
                                inputs: input.clone(),
                                outputs: HashSet::new(),
                            });
                        }

                        self.compute_and_apply_keys(&event.time)?;
                        self.cancel_pending_tap();
                    },
                    None => {
                        // Just pass it through
                        self.cancel_pending_tap();
                        self.compute_and_apply_keys(&event.time)?;
                    },
                }
            },
            KeyEventType::Repeat => {
                match self.lookup_mapping(code) {
                    Some(Mapping::DualRole { hold, .. }) => {
                        self.emit_keys(&hold, &event.time, KeyEventType::Repeat)?;
                    },
                    Some(Mapping::Remap { output, .. }) => {
                        let output: Vec<KeyCode> = output.iter().cloned().collect();
                        self.emit_keys(&output, &event.time, KeyEventType::Repeat)?;
                    },
                    Some(Mapping::ModeSwitch { .. }) => {
                        // Swallow repeats for mode switch chords
                    },
                    None => {
                        // If this key is suppressed due to a broken chord, swallow repeats
                        if self
                            .suppressed_until_released
                            .contains(&code)
                        {
                            // do nothing
                        } else {
                            // Just pass it through
                            self.cancel_pending_tap();
                            self.write_event_and_sync(event)?;
                        }
                    },
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
        // Keep only keys that are still physically held down
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

/// Orders modifier keys ahead of non-modifier keys.
/// Unfortunately the underlying type doesn't allow direct
/// comparison, but that's ok for our purposes.
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
