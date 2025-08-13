# Configuration (TOML)

Top-level fields (in `*.toml`):
- `device_name: Option<String>` — required unless provided via `--device-name`.
- `phys: Option<String>` — helps disambiguate multiple devices with the same name.
- `[[dual_role]]: Array` — dual-role mappings.
- `[[remap]]: Array` — simple remaps and chords.

Example:
```toml
# Identify the device
device_name = "AT Translated Set 2 keyboard"
# phys = "usb-0000:07:00.3-2.1.1/input0"

# Dual role: hold -> ctrl, tap -> esc
[[dual_role]]
input = "KEY_CAPSLOCK"
hold  = ["KEY_LEFTCTRL"]
tap   = ["KEY_ESC"]

# Remaps/chords
[[remap]]
input  = ["KEY_LEFTALT", "KEY_UP"]
output = ["KEY_PAGEUP"]
```

Semantics:
- __Key names__: must be valid `EV_KEY` codes (e.g., `KEY_*`). Parsing is via `evdev_rs::enums::EventCode::from_str`.
- __Order of application__:
  - All `DualRole` rules apply first.
  - `Remap` rules apply in file order.
- __Chord matching__ (`Remap`):
  - A rule matches if its `input` set is a subset of current keys (after DualRole transform) and not shadowed by prior rule outputs.
  - Non-modifier inputs/outputs are removed from the intermediate set to prevent subsequent rules from chaining on them.
- __Largest chord wins__:
  - If multiple `Remap` rules include the pressed key and all inputs are down, the rule with the largest `input` set is preferred.
- __Dual role timing__:
  - Tap threshold is 200ms (`src/remapper.rs::timeval_diff`). Quick press+release emits `tap`. Longer hold emits `hold` until release.

Notes:
- Extra fields in config are ignored by serde; `mode = "..."` in examples is currently unused.
- If `device_name` is omitted, it must be provided via CLI (`--device-name`).
- Use `evremap list-devices` to discover `device_name` and `phys` values.
