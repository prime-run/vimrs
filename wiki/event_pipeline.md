# Event Processing Pipeline

This document details the exact pipeline from physical key events to synthesized
output as implemented in `src/remapper.rs`. It focuses on state transitions,
lookup rules, suppression, timing, and emission ordering.

## Core types and state

- `InputMapper` owns devices and a `RemapEngine` state.
- `RemapEngine` mutable fields:
  - `input_state: HashMap<KeyCode, TimeVal>` — physical keys currently pressed (with press time).
  - `output_keys: HashSet<KeyCode>` — keys currently pressed on the uinput device.
  - `mappings: Vec<Mapping>` — ordered list (`DualRole`, `Remap`, `ModeSwitch`).
  - `tapping: Option<KeyCode>` — DualRole tap candidate (single-key).
  - `suppressed_until_released: HashSet<KeyCode>` — inputs to ignore until physical release.
  - `active_remaps: Vec<ActiveRemap>` — engaged remaps or switches.
  - `active_mode: Option<String>` — current mode; defaults to `Some("default")`.

## Loop and dispatch

```rust
loop {
    let (status, ev) = input.next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
    match status {
        ReadStatus::Success => {
            if let EventCode::EV_KEY(code) = ev.event_code {
                update_with_event(&ev, code)?;
            } else {
                output.write_event(&ev)?; // pass-through non-key
            }
        }
        ReadStatus::Sync => bail!("ReadStatus::Sync"),
    }
}
```

`update_with_event()` deserializes `KeyEventType` from the value and dispatches to `on_press`,
`on_release`, or `on_repeat` logic in-place.

## Press handling

- Insert `(code -> time)` into `input_state`.
- Drop any entries in `suppressed_until_released` no longer physically held in `input_state`.
- If `code` is in `suppressed_until_released`, swallow the press (no output), but keep state.
- `lookup_mapping_index(code)` under `active_mode` chooses the best matching mapping:
  - `DualRole { input == code }` wins immediately (exact match, if mode matches):
    - Push `ActiveRemap { kind=DualRole, inputs=[code], outputs=hold.., mode }`.
    - Set `tapping = Some(code)`.
    - `compute_and_apply_keys(time)`.
  - Else, among `Remap` whose `input` set contains `code` and is a subset of currently pressed
    physical keys, pick the largest chord; on ties, prefer `ModeSwitch` over `Remap`.
  - `ModeSwitch` candidate: if `scope` is `None` or equals `active_mode`, then:
    - Set `active_mode = Some(mode)`.
    - Push `ActiveRemap { kind=ModeSwitch, inputs, outputs=[], mode: Some(mode) }`.
    - Add all inputs of the switch to `suppressed_until_released`.
    - Clear pending `tapping`.
    - `compute_and_apply_keys(time)`.
- If no mapping matches:
  - Clear pending `tapping` (a non-dual-role press disqualifies a tap).
  - `compute_and_apply_keys(time)` to reflect physical press (subject to suppression and DualRole holds).

## Release handling

- Remove `code` from `input_state`.
- Prune `suppressed_until_released` for keys not physically held anymore.
- End any `active_remaps` whose inputs include `code`.
- If a `Remap` is ended while other chord members remain held, add those remaining non-modifier
  inputs to `suppressed_until_released` to prevent leakage of partial chords.
- `compute_and_apply_keys(time)`.
- DualRole tap check:
  - If `tapping == Some(code)` and `lookup_dual_role_index(code)` exists and the press duration
    `time - input_state_timestamp` is <= 200ms, emit the `tap` sequence immediately:
    - For each key `k` in `tap`: press `k`, sync, then release `k`, sync.
  - Clear `tapping` if it was `code`.

## Repeat handling

- If `code` is in `suppressed_until_released`, swallow the repeat.
- Try `emit_repeat_for_active_remap(code, time)`:
  - Identify the most specific active remap containing `code` and valid under `active_mode`.
  - If it’s DualRole, repeat the `hold` outputs.
  - If it’s Remap, repeat the `output` set.
  - ModeSwitch has no repeat.
- Otherwise fallback lookup:
  - `lookup_dual_role_index(code)` => repeat its `hold`.
  - Else `lookup_mapping_index(code)` => if Remap, repeat its `output`.
- Sync after repeats.

## Computing desired key set

```rust
fn compute_keys(&self) -> HashSet<KeyCode> {
    // 1) Start from physically held keys
    let mut keys = self.input_state.keys().cloned().collect::<HashSet<_>>();

    // 2) Remove suppressed
    for s in &self.suppressed_until_released { keys.remove(s); }

    // 3) DualRole: substitute holds for inputs under active mode
    for m in &self.mappings {
        if let Mapping::DualRole { input, hold, mode, .. } = m {
            let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(m), Some(a)) => m == a,
            };
            if mode_ok && keys.contains(input) {
                keys.remove(input);
                for h in hold { keys.insert(*h); }
            }
        }
    }

    // 4) Apply engaged Remaps under active mode (inputs removed, outputs added)
    for ar in &self.active_remaps {
        if ar.kind == ActiveKind::Remap {
            let mode_ok = match (ar.mode.as_ref(), self.active_mode.as_ref()) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(m), Some(a)) => m == a,
            };
            if mode_ok {
                for i in &ar.inputs { keys.remove(i); }
                for o in &ar.outputs { keys.insert(*o); }
            }
        }
    }

    keys
}
```

## Applying key diffs and ordering

- `compute_and_apply_keys(time)` compares desired keys vs `output_keys` and emits a minimal diff.
- Order matters due to modifiers:
  - Presses: modifiers first — `to_press.sort_by_key(|k| !is_modifier(*k))`.
  - Releases: modifiers last — `to_release.sort_by_key(|k| is_modifier(*k))`.
- Events are batched between `SYN_REPORT`s using `write_event_and_sync()`.

## Mode interactions

- `active_mode` gates DualRole substitutions and engaged Remap application.
- `ModeSwitch { scope }` can be global (`None`) or scoped to a specific mode.
- When a `ModeSwitch` chord engages, its input keys are added to `suppressed_until_released`.

## Suppression rules

- When a chord breaks (a member released) while other members remain held, the remaining
  physical inputs are added to `suppressed_until_released` until they are physically released.
- Suppression prevents unintended fallthrough presses or repeats.
- Suppressed keys swallow press/repeat; release is still processed to drop suppression.
