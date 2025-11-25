use crate::mapping::*;
use anyhow::*;
use evdev_rs::{Device, DeviceWrapper, GrabMode, InputEvent, ReadFlag, TimeVal, UInputDevice};
use std::collections::HashSet;
use std::path::Path;

use super::engine::RemapEngine;
use super::types::*;
use super::util::{is_modifier, make_event, timeval_diff};

pub struct InputMapper {
    input: Device,
    output: UInputDevice,
    state: RemapEngine,
}

fn enable_key_code(input: &mut Device, key: KeyCode) -> Result<()> {
    input.enable(EventCode::EV_KEY(key)).context(format!("enable key {key:?}"))?;
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
                Mapping::Macro { seq, .. } => {
                    let mut to_enable: HashSet<KeyCode> = HashSet::new();
                    for op in seq {
                        match op {
                            &crate::mapping::MacroOp::Press(k) | &crate::mapping::MacroOp::Release(k) => {
                                to_enable.insert(k);
                            },
                        }
                    }
                    for k in to_enable {
                        enable_key_code(&mut input, k)?;
                    }
                },
                Mapping::ModeSwitch { .. } => {},
            }
        }

        let output = UInputDevice::create_from_device(&input)
            .context(format!("creating UInputDevice from {}", path.display()))?;

        input.grab(GrabMode::Grab).context(format!("grabbing exclusive access on {}", path.display()))?;

        Ok(Self { input, output, state: RemapEngine::new(mappings) })
    }

    pub fn run_mapper(&mut self) -> Result<()> {
        log::info!("Going into read loop");
        loop {
            let (status, event) = self.input.next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
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
        let mut to_release: Vec<KeyCode> = self.state.output_keys.difference(&desired_keys).cloned().collect();
        let mut to_press: Vec<KeyCode> = desired_keys.difference(&self.state.output_keys).cloned().collect();

        if !to_release.is_empty() {
            to_release.sort_by_key(|k| is_modifier(*k));
            self.emit_keys(&to_release, time, KeyEventType::Release)?;
        }
        if !to_press.is_empty() {
            to_press.sort_by_key(|k| !is_modifier(*k));
            self.emit_keys(&to_press, time, KeyEventType::Press)?;
        }
        Ok(())
    }

    fn emit_repeat_for_active_remap(&mut self, code: KeyCode, time: &TimeVal) -> Result<bool> {
        let mut dual_idx: Option<usize> = None;
        let mut best_remap_idx: Option<usize> = None;
        let mut best_len: usize = 0;
        for (idx, ar) in self.state.active_remaps.iter().enumerate() {
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
            let len = self.state.active_remaps[idx].outputs_vec.len();
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
                    self.state.active_remaps.retain(|ar| !ar.inputs.contains(&code));
                    for inputs in ended_inputs {
                        for k in inputs {
                            if k != code && self.state.input_state.contains_key(&k) && !is_modifier(k) {
                                self.state.suppressed_until_released.insert(k);
                            }
                        }
                    }
                }

                self.compute_and_apply_keys(&event.time)?;

                let mut tap_keys: Option<Vec<KeyCode>> = None;
                if let Some(idx) = self.state.lookup_dual_role_index(code) && let Mapping::DualRole { tap, .. } = &self.state.mappings[idx] {
                    tap_keys = Some(tap.clone());
                }
                if let Some(tap_vec) = tap_keys
                    && let Some(tapping) = self.state.tapping.take()
                    && tapping == code
                    && timeval_diff(&event.time, &pressed_at) <= std::time::Duration::from_millis(200)
                {
                    self.emit_keys(&tap_vec, &event.time, KeyEventType::Press)?;
                    self.emit_keys(&tap_vec, &event.time, KeyEventType::Release)?;
                }
            },

            KeyEventType::Press => {
                self.state.input_state.insert(code, event.time);
                self.state.prune_suppressed_keys();

                match self.state.lookup_mapping_index(code) {
                    Some(idx) => match &self.state.mappings[idx] {
                        Mapping::DualRole { .. } => {
                            let (inputs_set, outputs_set, outputs_vec, mode_clone) = {
                                if let Mapping::DualRole { hold, mode, .. } = &self.state.mappings[idx] {
                                    let mut s: HashSet<KeyCode> = HashSet::new();
                                    s.insert(code);
                                    let vec = hold.clone();
                                    let set: HashSet<KeyCode> = hold.iter().cloned().collect();
                                    (s, set, vec, mode.clone())
                                } else { unreachable!() }
                            };

                            if !self.state.active_remaps.iter().any(|ar| ar.inputs == inputs_set) {
                                self.state.active_remaps.push(ActiveRemap {
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
                                if let Mapping::Remap { input, output, mode, .. } = &self.state.mappings[idx] {
                                    (input.clone(), output.clone(), output.iter().cloned().collect::<Vec<KeyCode>>(), mode.clone())
                                } else { unreachable!() }
                            };

                            if !self.state.active_remaps.iter().any(|ar| ar.inputs == input_set) {
                                self.state.active_remaps.push(ActiveRemap {
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
                                if let Mapping::ModeSwitch { input, mode, .. } = &self.state.mappings[idx] {
                                    let s: HashSet<KeyCode> = input.clone();
                                    let v: Vec<KeyCode> = s.iter().cloned().collect();
                                    (v, s, mode.clone())
                                } else { unreachable!() }
                            };

                            for k in &inputs_vec {
                                self.state.suppressed_until_released.insert(*k);
                            }

                            self.state.active_mode = Some(mode_new);

                            if !self.state.active_remaps.iter().any(|ar| ar.inputs == inputs_set) {
                                self.state.active_remaps.push(ActiveRemap {
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
                        Mapping::Macro { .. } => {
                            let (inputs_vec, inputs_set, seq_ops) = {
                                if let Mapping::Macro { input, seq, .. } = &self.state.mappings[idx] {
                                    let s: HashSet<KeyCode> = input.clone();
                                    let v: Vec<KeyCode> = s.iter().cloned().collect();
                                    (v, s, seq.clone())
                                } else { unreachable!() }
                            };

                            for k in &inputs_vec {
                                self.state.suppressed_until_released.insert(*k);
                            }

                            // Immediate macro; no ActiveRemap
                            self.emit_macro(&seq_ops, &event.time)?;
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
                                Mapping::DualRole { hold, .. } => { to_emit = Some(hold.clone()); },
                                Mapping::Remap { output, .. } => { to_emit = Some(output.iter().cloned().collect()); },
                                Mapping::Macro { .. } => { /* no repeat */ },
                                Mapping::ModeSwitch { .. } => {},
                            }
                            if let Some(vec) = to_emit {
                                self.emit_keys(&vec, &event.time, KeyEventType::Repeat)?;
                            }
                        },
                        None => {
                            if self.state.suppressed_until_released.contains(&code) {
                            } else {
                                self.state.cancel_pending_tap();
                                self.write_event_and_sync(event)?;
                            }
                        },
                    }
                }
            },
            KeyEventType::Unknown(_) => { self.write_event_and_sync(event)?; },
        }

        Ok(())
    }

    fn emit_keys(&mut self, key: &[KeyCode], time: &TimeVal, event_type: KeyEventType) -> Result<()> {
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
                KeyEventType::Press | KeyEventType::Repeat => { self.state.output_keys.insert(*key); },
                KeyEventType::Release => { self.state.output_keys.remove(key); },
                _ => {},
            }
        }
        Ok(())
    }

    fn emit_macro(&mut self, ops: &[crate::mapping::MacroOp], time: &TimeVal) -> Result<()> {
        for op in ops {
            match *op {
                crate::mapping::MacroOp::Press(k) => {
                    let event = make_event(k, time, KeyEventType::Press);
                    self.write_event(&event)?;
                },
                crate::mapping::MacroOp::Release(k) => {
                    let event = make_event(k, time, KeyEventType::Release);
                    self.write_event(&event)?;
                },
            }
        }
        self.generate_sync_event(time)?;
        Ok(())
    }

    fn generate_sync_event(&self, time: &TimeVal) -> Result<()> {
        self.output.write_event(&InputEvent::new(
            time,
            &EventCode::EV_SYN(evdev_rs::enums::EV_SYN::SYN_REPORT),
            0,
        ))?;
        Ok(())
    }
}
