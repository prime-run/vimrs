# Modules Overview

This repo is intentionally small. Each module has a focused responsibility.

## `src/main.rs`

- __CLI enum `Opt`__: `ListDevices`, `ListKeys`, `DebugEvents { device_name, phys }`, `Remap { config_file, delay, device_name, phys, wait_for_device }`.
- __Logging__: `setup_logger()` uses `env_logger` with `EVREMAP_LOG` and `EVREMAP_LOG_STYLE`; default filter `Info`.
- __Device acquisition__: `get_device(name, phys, wait_for_device)` optionally polls until found with backoff (1s..10s).
- __Debug events__: `debug_events(DeviceInfo)` prints `EV_KEY` codes and values (0/1/2).
- __List keys__: `list_keys()` constructs all `EV_KEY` values (printing currently commented out).

## `src/deviceinfo.rs`

- __`DeviceInfo`__: `{ name: String, path: PathBuf, phys: String }`.
- __`with_path(path)`__: opens a single `/dev/input/event*` node and reads `name`/`phys`.
- __`with_name(name, phys)`__: selects device by `phys` if provided; else first device whose `name` matches. Warns if multiple match.
- __`list_devices()`__: prints Name/Path/Phys for all devices.
- __`obtain_device_list()`__ (private): scans `/dev/input`, opens each `event*` file, sorts by `(name, event_number)`.

## `src/mapping.rs`

- __Key alias__: `type KeyCode = evdev_rs::enums::EV_KEY`.
- __`MappingConfig`__: `{ device_name: Option<String>, phys: Option<String>, mappings: Vec<Mapping> }`.
  - `from_file(path)`: reads TOML into `ConfigFile`, converts `[[dual_role]]` then `[[remap]]` into `Vec<Mapping>` (dual-role entries always precede remaps).
- __`Mapping` enum__:
  - `DualRole { input: KeyCode, hold: Vec<KeyCode>, tap: Vec<KeyCode> }`
  - `Remap { input: HashSet<KeyCode>, output: HashSet<KeyCode> }`
- __Deserialization__: `KeyCodeWrapper` implements `TryFrom<String>` via `EventCode::from_str(EventType::EV_KEY, s)`; invalid keys -> `ConfigError::InvalidKey`.
- __Config structs__: `DualRoleConfig`, `RemapConfig`, `ConfigFile` with `#[serde(default)]` on arrays/optionals.

## `src/remapper.rs`

- __`InputMapper` state__:
  - `input: Device`, `output: UInputDevice`
  - `input_state: HashMap<KeyCode, TimeVal>` (keys currently down + when pressed)
  - `output_keys: HashSet<KeyCode>` (keys currently down on virtual device)
  - `mappings: Vec<Mapping>`, `tapping: Option<KeyCode>`
- __Construction__: `create_mapper(path, mappings)`
  - Enables all mapped output key codes on an in-memory `Device` using `enable(EventCode::EV_KEY(..))`.
  - Creates `UInputDevice` via `UInputDevice::create_from_device(&input)`.
  - Grabs the real device with `GrabMode::Grab`.
- __Main loop__: `run_mapper()`
  - Reads events with `ReadFlag::NORMAL | ReadFlag::BLOCKING`.
  - Pass-through non-key events; key events -> `update_with_event()`.
- __Core transforms__:
  - `compute_keys()` applies all `DualRole` first (replace `input` with `hold`), then applies `Remap` rules (set-logic; see `event_pipeline.md`).
  - `compute_and_apply_keys(time)` diffs `desired_keys` vs `output_keys`, then emits presses (modifiers first) and releases (modifiers last).
- __Lookup helpers__:
  - `lookup_dual_role_mapping(code)` exact match.
  - `lookup_mapping(code)` returns first `DualRole` match or the largest-chord `Remap` candidate whose inputs are all down.
- __Tap/Hold__:
  - 200ms threshold via `timeval_diff()`. On quick release of a `DualRole` input and if it is the current `tapping` key, emit `tap` press+release.
- __Repeat handling__: for `Repeat`, emits `hold` (DualRole) or `output` (Remap); otherwise pass-through.
- __Helpers__: `KeyEventType`, `timeval_diff`, `is_modifier`, `modifiers_first/last`, `make_event`, `generate_sync_event`.
