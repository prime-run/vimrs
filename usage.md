# Usage: Configuration Style

This guide shows the TOML structure supported by evremap and recommended patterns.
See the implementation in `src/mapping.rs` and `src/remapper.rs` for exact behavior.

- Keys are Linux input names like `KEY_A`, `KEY_LEFTCTRL`, `KEY_HOME`.
- Modes let you scope mappings; the implicit default mode is `"default"`.
- You can map either:
  - Simple remaps with `output` (pressed while input is held), or
  - Sequences with `output_sequence` (precompiled macro phases).

## Top-level structure

```toml
# Optional device identifiers
device_name = "Your Keyboard Name"    # optional
phys = "usb-0000:00:14.0-6/input0"   # optional

# Global mappings (apply to the implicit "default" mode unless `mode` is set)
[[remap]]
input = ["KEY_SEMICOLON"]
output = ["KEY_ESC"]

# Global mode switch: active in any mode
[[mode_switch]]
input = ["KEY_LEFTALT", "KEY_V"]
mode = "normalvim"

# Start a named mode section
[modes.normalvim]
exclusive = false  # if true, unspecified keys are no-ops in this mode

# Mode-scoped switch (only works while in this mode)
[[modes.normalvim.switch]]
input = ["KEY_LEFTALT", "KEY_V"]
mode = "default"
```

## Remaps vs. Sequences

- `[[remap]]` and `[[modes.NAME.remap]]` share the same fields:
  - `input`: array of one or more keys (the chord that triggers the mapping)
  - `output`: array of keys that behave like you’re holding them (press on trigger, release on input release)
  - `output_sequence`: array of phases; when present, the mapping becomes a macro sequence
  - `mode` (top-level only): if omitted, defaults to `"default"`

Use one of `output` or `output_sequence` per mapping.

### Sequence phases (macros)

Each phase supports two lists:
- `hold`: keys to press at the start of the phase
- `tap`: keys to press+release in order

At the end of each phase, evremap releases all `hold` keys (in reverse order). Holds do not persist across phases.

Example:

```toml
[[modes.normalvim.remap]]
input = ["KEY_LEFTSHIFT", "KEY_D"]
output_sequence = [
  { hold = ["KEY_LEFTSHIFT"], tap = ["KEY_END"] },
  { tap = ["KEY_ESC"] },
]
```

## Dual role

```toml
[[dual_role]]
input = "KEY_SPACE"                # single key
hold  = ["KEY_LEFTCTRL"]           # emitted while held
tap   = ["KEY_ESC"]                # emitted on quick tap
```

Mode-scoped dual roles:

```toml
[modes.coding]
exclusive = true

[[modes.coding.dual_role]]
input = "KEY_CAPSLOCK"
hold  = ["KEY_LEFTCTRL"]
tap   = ["KEY_ESC"]
```

## Mode switching

- Global `[[mode_switch]]`: active in every mode.
- Mode-scoped `[[modes.NAME.switch]]`: only active while in that mode.
- Safety: if you do NOT define any global switch back to `default`, evremap injects a global escape chord `KEY_LEFTCTRL + KEY_BACKSLASH` to return to `default`.

Examples:

```toml
# Global toggle into a mode
[[mode_switch]]
input = ["KEY_LEFTALT", "KEY_V"]
mode = "normalvim"

[modes.normalvim]
exclusive = false

# Scoped toggle back to default
[[modes.normalvim.switch]]
input = ["KEY_LEFTALT", "KEY_V"]
mode = "default"
```

## Examples

### Simple word-motions in a mode

```toml
[modes.normalvim]
exclusive = false

# w => Ctrl+Right
[[modes.normalvim.remap]]
input = ["KEY_W"]
output_sequence = [ { hold = ["KEY_LEFTCTRL"], tap = ["KEY_RIGHT"] } ]

# b => Ctrl+Left
[[modes.normalvim.remap]]
input = ["KEY_B"]
output_sequence = [ { hold = ["KEY_LEFTCTRL"], tap = ["KEY_LEFT"] } ]
```

### Multi-phase macro

```toml
# Shift+D: select to end, then escape
[[modes.normalvim.remap]]
input = ["KEY_LEFTSHIFT", "KEY_D"]
output_sequence = [
  { hold = ["KEY_LEFTSHIFT"], tap = ["KEY_END"] },
  { tap = ["KEY_ESC"] },
]
```

## Style tips

- Prefer short, app-agnostic chords for switching modes.
- In exclusive modes, explicitly allow keys you still need (ESC to escape, etc.).
- Keep macros deterministic; avoid relying on editor-specific word rules unless you’re OK to tweak taps.
- Don’t duplicate the same `input` chord within the same scope.

## Troubleshooting

- Unknown key names: run `evremap list-keys`.
- Got “stuck” in a mode: press `LeftCtrl + Backslash` (built-in escape) or add your own global switch to `default`.
- Macro didn’t land the cursor as expected: editors differ on word boundaries. Adjust number of arrow/word taps.
