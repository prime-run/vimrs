# Configuration guide

This document explains two key behaviors in evremap's config:
- exclusive modes (per-mode key blocking)
- the emergency escape to the default mode

See the implementation in `src/mapping.rs` and `src/remapper.rs` for exact details.

## Exclusive modes

When a mode is marked `exclusive = true`, evremap generates implicit no-op remaps for every key not explicitly referenced in that mode. Practically, this “blocks” all unspecified keys for that mode.

- Keys considered “allowed” are those that appear in that mode's:
  - `dual_role.input`
  - `remap.input` (including chords)
  - `switch.input` (mode switches within that mode)
- For every other EV_KEY, a `Remap` with empty output is generated for that mode.

### Important limitation (current engine precedence)

The remapping engine currently prioritizes `DualRole` mappings during lookup. This means:
- A global `[[dual_role]]` on some key can still fire even in an exclusive mode for that key.
- In other words, exclusivity is “best effort” if a matching global DualRole exists.

Ways to avoid surprises:
- Move global `[[dual_role]]` entries into `modes.default` if you don't want them active in other modes.
- Avoid defining `[[dual_role]]` on keys you intend to fully block in other modes.

### Minimal example

```toml
# Top-level section

device_name = "Your Keyboard"

# Top-level remap: defaults to mode="default" unless you set `mode`
[[remap]]
input = ["KEY_LEFTALT", "KEY_J"]
output = ["KEY_DOWN"]

# Mode-specific: gaming
[modes.gaming]
exclusive = true

# Allow W -> Up in gaming mode
[[modes.gaming.remap]]
input = ["KEY_W"]
output = ["KEY_UP"]

# Allow ESC to return to default mode
[[modes.gaming.switch]]
input = ["KEY_ESC"]
mode = "default"
```

With `exclusive = true`, only keys that appear in `modes.gaming` mappings are allowed while in `gaming`. All others are blocked by generated no-op remaps. If you also have a global `[[dual_role]]` for `KEY_W`, its DualRole behavior may still apply; scope it to `modes.default` if you want it inactive in `gaming`.

## Emergency escape to default mode

To prevent accidental lock-in, evremap injects a global escape chord to the `default` mode if you didn't define any global switch to `default`:

- Injected chords:
  - `KEY_LEFTCTRL + KEY_BACKSLASH` → `default`
  - `KEY_RIGHTCTRL + KEY_BACKSLASH` → `default`
- This injection is appended after your mappings are parsed so that your own global switches take precedence on ties.

### Overriding or disabling the injected escape

- If you add any global `[[mode_switch]]` with `mode = "default"`, evremap will NOT inject the fallback at all.
- You can choose another chord you prefer and the built-in fallback won't be added.

Example override:

```toml
# Define your own global escape; built-in Ctrl+\ will not be injected
[[mode_switch]]
input = ["KEY_LEFTCTRL", "KEY_GRAVE"]
mode = "default"
```

### Pitfall: DualRole can shadow the escape

Because `DualRole` mappings are considered first in the current engine, if you bind `KEY_BACKSLASH` (or any key used as the last-pressed key in your escape chord) as a `[[dual_role]]` globally, it might prevent the escape chord from triggering when that key is pressed last.

Mitigations:
- Pick an escape chord whose keys are not used as global DualRoles; or
- Scope those DualRoles to `modes.default`; or
- Define your own `[[mode_switch]]` escape to a different chord (see example above).

## Mode-scoped mappings recap

- Global tables:
  - `[[dual_role]]` — applies in all modes
  - `[[remap]]` — defaults to `mode="default"` unless you set `mode`
  - `[[mode_switch]]` — global switch/escape
- Mode-scoped tables (only within that mode):
  - `[modes.NAME]` to start a mode section
  - `[[modes.NAME.dual_role]]`
  - `[[modes.NAME.remap]]`
  - `[[modes.NAME.switch]]`

See `examples/` for more TOML samples. If something doesn't behave as expected, check precedence details in `src/remapper.rs` and the limitation above.
