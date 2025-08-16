# Modules Overview

This is a developer-facing overview of the codebase. It summarizes responsibilities,
key types, and important functions per module. See `architecture.md` for data flow and
`event_pipeline.md` for algorithms.

## `src/main.rs`

- __CLI (`Opt`)__
  - `ListDevices`
  - `ListKeys`
  - `DebugEvents { --device-name <str>, --phys <str?> }`
  - `Remap { <CONFIG-FILE>, --delay <f64>, --device-name <str?>, --phys <str?>, --wait-for-device }`
- __Logger__: `setup_logger()` uses `env_logger` with defaults; env overrides via `EVREMAP_LOG` and `EVREMAP_LOG_STYLE`.
- __Key listing__: `list_keys()` builds all `EV_KEY` codes and sorts (actual printing is not performed).
- __Device resolution__: `get_device(name, phys, wait_for_device)` optionally polls (1s backoff up to 10s) until attached.
- __Debug events__: `debug_events(DeviceInfo)` prints key events (code, value) from the physical device.
- __Remap run__: loads `MappingConfig`, applies CLI overrides for `device_name`/`phys`, delays briefly, resolves device, then starts `InputMapper`.

## `src/mapping.rs`

- __Key alias__: `type KeyCode = evdev_rs::enums::EV_KEY` (re-exported as `EV_KEY as KeyCode`).
- __`MappingConfig`__: `{ device_name: Option<String>, phys: Option<String>, mappings: Vec<Mapping> }`.
  - `from_file(path)`: parses TOML and produces a flat ordered `mappings` vector from:
    - top-level `[[dual_role]]`, `[[remap]]`, `[[mode_switch]]`
    - per-mode `[modes.<name>]` sections, lifting entries into `mode=Some(name)` (and ModeSwitch `scope=Some(name)`).
- __`Mapping` enum__ (current):
  - `DualRole { input: KeyCode, hold: Vec<KeyCode>, tap: Vec<KeyCode>, mode: Option<String> }`
  - `Remap { input: HashSet<KeyCode>, output: HashSet<KeyCode>, mode: Option<String> }`
  - `ModeSwitch { input: HashSet<KeyCode>, mode: String, scope: Option<String> }`
- __Config parsing__:
  - `KeyCodeWrapper: TryFrom<String>` via `EventCode::from_str(EventType::EV_KEY, s)`.
  - `ConfigError::{InvalidKey, ImpossibleParseKey}` with helpful messages.
  - `RemapConfig.mode` defaults to `Some("default")` when unspecified.
  - `ModeSection` maps `[[modes.<name>.switch]]` into `switch_to` with `#[serde(rename = "switch")]`.

## `src/remapper.rs`

- __`InputMapper`__
  - Fields: `input: Device`, `output: UInputDevice`, `state: RemapEngine`.
  - `create_mapper(path, mappings)`:
    - Opens the physical device, sets a descriptive uinput name, enables all required output key codes
      from `DualRole.tap`, `DualRole.hold`, and `Remap.output`, creates `UInputDevice`, and grabs the real device.
  - `run_mapper()`:
    - Blocking loop: reads events (`ReadFlag::NORMAL|BLOCKING`), passes through non-`EV_KEY`, and calls `update_with_event()` for keys.

- __`RemapEngine`__ (mutable state)
  - `input_state: HashMap<KeyCode, TimeVal>` — keys currently down with press times.
  - `output_keys: HashSet<KeyCode>` — keys currently down on the uinput device.
  - `mappings: Vec<Mapping>` — ordered, flat list.
  - `tapping: Option<KeyCode>` — DualRole tap candidate.
  - `suppressed_until_released: HashSet<KeyCode>` — non-modifier inputs suppressed until they are released after a broken chord.
  - `active_remaps: Vec<ActiveRemap>` — engaged remaps/switches with inputs/outputs and kind.
  - `active_mode: Option<String>` — current mode; initialized to `Some("default")`.

- __Key helpers__
  - `KeyEventType` enum with `from_value()` and `value()`.
  - `timeval_diff(newer, older) -> Duration` (200ms threshold used for tap detection).
  - `is_modifier(KeyCode) -> bool` (FN, ALT, META, CTRL, SHIFT variants).
  - `make_event()`, `write_event()`, `write_event_and_sync()`, `generate_sync_event()`.

- __Core methods__
  - `update_with_event(&mut self, event: &InputEvent, code: KeyCode) -> Result<()>` — press/release/repeat logic with suppression and active remaps.
  - `compute_and_apply_keys(&mut self, time)` — diffs desired vs. actual output, sorts presses/releases by `is_modifier`, emits batched events.
  - `emit_repeat_for_active_remap(&mut self, code, time)` — repeats outputs for the most specific engaged remap or DualRole.

- __Lookup__
  - `lookup_dual_role_index(code)` — exact DualRole match under the current mode.
  - `lookup_mapping_index(code)` — prefers DualRole; else largest-chord Remap including `code` under the current mode; ModeSwitch wins on chord-size ties with valid `scope`.

## `src/deviceinfo.rs`

- __`DeviceInfo { name, path, phys }`__ accessors of a physical device.
- `with_path(path)` — open one `/dev/input/event*` and extract info.
- `with_name(name, phys)` — return by `phys` when set; else first by `name` (warn on multiple matches with guidance to specify `phys`).
- `list_devices()` — prints Name/Path/Phys for all sorted devices.
- Internals: `obtain_device_list()` scans `/dev/input`, opens `event*`, sorts by `(name, event_number)` using `event_number_from_path()`.
