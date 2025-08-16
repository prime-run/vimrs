# Architecture

Evremap is a single-binary tool that remaps Linux input events at the evdev layer.
This wiki is for contributors. It documents module boundaries, data flow, and
key algorithms, mirroring the current code in `src/`.

- Entrypoint: `src/main.rs`
- Model & config: `src/mapping.rs`
- Engine: `src/remapper.rs`
- Device discovery: `src/deviceinfo.rs`

## High-level flow

1. CLI (`Opt` in `src/main.rs`) selects subcommand.
2. `MappingConfig::from_file(path)` loads TOML and produces a flat, ordered `Vec<Mapping>`
   annotated with per-mode membership and `ModeSwitch` scopes.
3. `get_device()` resolves a device path by `name` and optional `phys` (with optional polling).
4. `InputMapper::create_mapper(path, mappings)` creates a uinput device mirroring required
   output capabilities, then exclusively grabs the physical device.
5. `InputMapper::run_mapper()` reads events, transforms keys via the `RemapEngine`, and writes
   synthetic events to the uinput device with `SYN_REPORT` batching.

## Data model (`src/mapping.rs`)

```rust
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Mapping {
    DualRole {
        input: KeyCode,
        hold: Vec<KeyCode>,
        tap: Vec<KeyCode>,
        mode: Option<String>,
    },
    Remap {
        input: HashSet<KeyCode>,
        output: HashSet<KeyCode>,
        mode: Option<String>,
    },
    ModeSwitch {
        input: HashSet<KeyCode>,
        mode: String,
        scope: Option<String>,
    },
}
```

- Top-level `[[dual_role]]`, `[[remap]]`, `[[mode_switch]]` map to `Mapping` with `mode=None`/`scope=None`.
- `[modes.<name>]` section lifts mode membership into `mode=Some(name)` and ModeSwitch `scope=Some(name)`.
- `Key` parsing: `KeyCodeWrapper: TryFrom<String>` via `EventCode::from_str(EventType::EV_KEY, s)`.
- Invalid keys raise `ConfigError::InvalidKey` with a hint to run `list-keys`.

## Engine & state (`src/remapper.rs`)

`InputMapper` embeds a `RemapEngine` that holds all mutable state:

- `input_state: HashMap<KeyCode, TimeVal>` — keys currently down with press timestamps.
- `output_keys: HashSet<KeyCode>` — keys currently held down on the uinput device.
- `mappings: Vec<Mapping>` — ordered flat list (DualRole / Remap / ModeSwitch).
- `tapping: Option<KeyCode>` — current DualRole tap candidate.
- `suppressed_until_released: HashSet<KeyCode>` — non-modifier inputs suppressed until released after a broken chord.
- `active_remaps: Vec<ActiveRemap>` — engaged DualRole/Remap/ModeSwitch (inputs, outputs, outputs_vec, kind, mode).
- `active_mode: Option<String>` — current logical mode; initialized to `Some("default")`.

### Event loop

```rust
impl InputMapper {
    pub fn run_mapper(&mut self) -> Result<()> {
        loop {
            let (status, event) = self.input.next_event(ReadFlag::NORMAL | ReadFlag::BLOCKING)?;
            match status {
                ReadStatus::Success => {
                    if let EventCode::EV_KEY(key) = event.event_code {
                        self.update_with_event(&event, key)?;
                    } else {
                        self.output.write_event(&event)?; // pass-through non-key
                    }
                }
                ReadStatus::Sync => bail!("ReadStatus::Sync!"),
            }
        }
    }
}
```

### Press / Release / Repeat

- Press:
  - Insert into `input_state`; prune `suppressed_until_released` for keys no longer held.
  - `lookup_mapping_index(code)` selects the best candidate under `active_mode`:
    - If `DualRole`: register `ActiveRemap` (kind=DualRole), recompute/apply keys, set `tapping=code`.
    - If `Remap`: register `ActiveRemap` (kind=Remap), recompute/apply keys, set `tapping=code`.
    - If `ModeSwitch`: set `active_mode`, add all its inputs to `suppressed_until_released`,
      register as active (kind=ModeSwitch), recompute/apply keys, cancel pending tap.
  - Else: cancel pending tap and recompute/apply keys.

- Release:
  - Remove from `input_state`; prune `suppressed_until_released`.
  - End `active_remaps` containing this key and suppress the remaining, still-held, non-modifier inputs.
  - Recompute/apply keys.
  - If the released key is a DualRole `input` and was the `tapping` key and the press lasted <= 200ms,
    emit its `tap` sequence (press + release) immediately.

- Repeat:
  - Prefer `emit_repeat_for_active_remap(code)` — repeats outputs from the most specific active remap
    (or the DualRole hold) that contains `code` and matches `active_mode`.
  - Otherwise fall back to lookup-based repeat: DualRole emits `hold`, Remap emits `output`, ModeSwitch none.
  - Repeats for suppressed keys are swallowed.

### Key set computation

```rust
impl RemapEngine {
    fn compute_keys(&self) -> HashSet<KeyCode> {
        let mut keys: HashSet<KeyCode> = self.input_state.keys().cloned().collect();
        for s in &self.suppressed_until_released { keys.remove(s); }
        // DualRole pass under active_mode
        for map in &self.mappings {
            if let Mapping::DualRole { input, hold, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && keys.contains(input) {
                    keys.remove(input);
                    for h in hold { keys.insert(*h); }
                }
            }
        }
        // Apply engaged Remaps under active_mode
        for ar in &self.active_remaps {
            if ar.kind == ActiveKind::Remap {
                let mode_ok = match (ar.mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok { for i in &ar.inputs { keys.remove(i); } for o in &ar.outputs { keys.insert(*o); } }
            }
        }
        keys
    }
}
```

### Ordering and modifiers

- Press ordering: press modifiers first: `to_press.sort_by_key(|k| !is_modifier(*k))`.
- Release ordering: release modifiers last: `to_release.sort_by_key(|k| is_modifier(*k))`.
- Modifiers are defined in `is_modifier(KeyCode)` and include `FN`, `ALT`, `META`, `CTRL`, `SHIFT` variants.

### Lookup rules

```rust
fn lookup_dual_role_index(&self, code: KeyCode) -> Option<usize>;
fn lookup_mapping_index(&self, code: KeyCode) -> Option<usize>;
```
- DualRole takes precedence when the pressed key equals its `input` and the mode matches.
- Among Remaps including the pressed key whose inputs are all currently held, the largest chord wins.
- ModeSwitch candidates are prioritized over Remaps when chord size ties (higher internal priority).
- ModeSwitch `scope` must match the current `active_mode` (or be `None`).

## Device discovery (`src/deviceinfo.rs`)

- `DeviceInfo { name, path, phys }` from `/dev/input/event*`.
- `with_name(name, phys)` returns a device by exact `phys` or first matching `name` (warns when multiple).
- `obtain_device_list()` scans `/dev/input`, opens each `event*`, and sorts by `(name, event_number)`.
- `list_devices()` prints device info for debugging.

## Logging & errors

- Logging via `log` facade and `env_logger`. Default `Info`; `EVREMAP_LOG` and `EVREMAP_LOG_STYLE` control level/style.
- Application errors are `anyhow::*`; config parse uses a typed `ConfigError`.
