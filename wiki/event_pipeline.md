# Event Pipeline & Algorithms

This describes how hardware events are transformed into output events.

## Sources and Sinks

- Input: `evdev_rs::Device` reading from `/dev/input/event*`.
- Output: `evdev_rs::UInputDevice` created from input capabilities.
- Sync: `SYN_REPORT` after each batch write.

## Event loop (`InputMapper::run_mapper`)

1. Read next event with `ReadFlag::NORMAL | BLOCKING`.
2. If not `EV_KEY`: pass-through via `write_event_and_sync`.
3. If `EV_KEY`: call `update_with_event(event, key)`.

## Internal state (`InputMapper`)

- `input_state: HashMap<KeyCode, TimeVal>` — keys currently down (press time).
- `output_keys: HashSet<KeyCode>` — keys currently down on virtual device.
- `mappings: Vec<Mapping>` — ordered: all DualRole entries, then Remap entries (from `MappingConfig`).
- `tapping: Option<KeyCode>` — candidate for 200ms tap.

## Key handling (`update_with_event`)

- Press:
  - Record `input_state[key] = time`.
  - If any mapping matches (`lookup_mapping`), compute/apply desired keys and mark `tapping = key`.
  - Else: cancel pending tap, compute/apply (effectively pass-through).
- Release:
  - Remove from `input_state`.
  - Recompute/apply desired keys.
  - If a DualRole exists for this key and release within 200ms, emit `tap` as press+release.
- Repeat:
  - DualRole: emit `hold` with Repeat.
  - Remap: emit `output` with Repeat.
  - None: pass-through.

## Computing desired keys (`compute_keys`)

Two passes over `self.mappings`:

1. DualRole pass:
   - If `input` is in current `keys`, replace it with all `hold` keys.
2. Remap pass (file order):
   - Maintain `keys_minus_remapped` (copy of keys without non-modifier inputs/outputs that applied).
   - If `input ⊆ keys_minus_remapped`:
     - Remove `input` from `keys` (non-modifiers from both `keys` and `keys_minus_remapped`).
     - Insert all `output` into `keys` and remove non-modifier outputs from `keys_minus_remapped`.

Notes:

- This prevents chaining on generated outputs unless they are modifiers.
- Ordering of `[[remap]]` in the file matters (earlier rules can shadow later ones).

## Applying differences (`compute_and_apply_keys`)

- Compute set diff between `desired_keys` and `output_keys`.
- Release extra keys first (sorted with `modifiers_last`).
- Press missing keys (sorted with `modifiers_first`).
- Emit `SYN_REPORT` once after each batch (`generate_sync_event`).

## Tap vs Hold

- Threshold: 200ms (`timeval_diff`).
- On quick release of a DualRole `input` and if it is the current `tapping` key, emit `tap` press+release.
- Longer holds are reflected through `compute_keys` as `hold` contributions.

## Lookup rules

- `lookup_dual_role_mapping(code)` — exact key match, highest precedence.
- `lookup_mapping(code)` — returns DualRole if exact match, else among matching Remap chords that include `code`, pick the one with the largest `input` set.

## Modifiers

- `is_modifier` identifies standard modifiers. They get priority ordering:
  - Press: modifiers first.
  - Release: modifiers last.

## Rustdoc-style snippets

```rust
// src/remapper.rs (signatures)
use anyhow::Result;
use evdev_rs::{InputEvent, TimeVal};
use std::collections::HashSet;

impl crate::remapper::InputMapper {
    pub fn run_mapper(&mut self) -> Result<()>;
    pub fn update_with_event(&mut self, event: &InputEvent, code: crate::mapping::KeyCode) -> Result<()>;
    fn compute_keys(&self) -> HashSet<crate::mapping::KeyCode>;
    fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()>;
}
```

```rust
// Ordering example: press modifiers first, release modifiers last
// src/remapper.rs (excerpt)
fn compute_and_apply_keys(&mut self, time: &TimeVal) -> Result<()> {
    let desired = self.compute_keys();
    let mut to_release: Vec<_> = self.output_keys.difference(&desired).cloned().collect();
    let mut to_press: Vec<_> = desired.difference(&self.output_keys).cloned().collect();
    to_release.sort_by(modifiers_last);
    self.emit_keys(&to_release, time, KeyEventType::Release)?;
    to_press.sort_by(modifiers_first);
    self.emit_keys(&to_press, time, KeyEventType::Press)?;
    Ok(())
}
```

```rust
// Tap vs hold (200ms threshold) — src/remapper.rs (excerpt)
use std::time::Duration;
if let Some(Mapping::DualRole { tap, .. }) = self.lookup_dual_role_mapping(code) {
    if let Some(tapping) = self.tapping.take() {
        if tapping == code && timeval_diff(&event.time, &pressed_at) <= Duration::from_millis(200) {
            self.emit_keys(&tap, &event.time, KeyEventType::Press)?;
            self.emit_keys(&tap, &event.time, KeyEventType::Release)?;
        }
    }
}
