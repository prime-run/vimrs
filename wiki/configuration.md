# Configuration (TOML)

Top-level fields (in `*.toml`):

- `device_name: Option<String>` — required unless provided via `--device-name`.
- `phys: Option<String>` — helps disambiguate multiple devices with the same name.
- `[[dual_role]]: Array` — global dual-role mappings (apply in all modes).
- `[[remap]]: Array` — default-mode remaps (treated as `modes.default.remap`).
- `[modes.<name>]` — per-mode section.
  - `[[modes.<name>.dual_role]]` — dual-role rules for that mode only.
  - `[[modes.<name>.remap]]` — remaps for that mode only.
  - `[[modes.<name>.switch]]` — chords that switch to some target mode (defined under the current mode scope).

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

# ----- Default mode -----
[modes.default]

[[modes.default.dual_role]]
input = "KEY_CAPSLOCK"
hold  = ["KEY_LEFTCTRL"]
tap   = ["KEY_ESC"]

[[modes.default.remap]]
input  = ["KEY_J"]
output = ["KEY_DOWN"]

# Multiple keymaps to switch out of default
[[modes.default.switch]]
input = ["KEY_LEFTALT", "KEY_N"]
mode  = "nav"

[[modes.default.switch]]
input = ["KEY_LEFTALT", "KEY_E"]
mode  = "edit"

# Explicit switch back to default (optional)
[[modes.default.switch]]
input = ["KEY_LEFTALT", "KEY_D"]
mode  = "default"

# ----- Nav mode -----
[modes.nav]

[[modes.nav.remap]]
input  = ["KEY_H"]
output = ["KEY_LEFT"]

[[modes.nav.remap]]
input  = ["KEY_L"]
output = ["KEY_RIGHT"]

# Define switches available while in nav mode
[[modes.nav.switch]]
input = ["KEY_LEFTALT", "KEY_D"]
mode  = "default"
```

Semantics:

- __Key names__: must be valid `EV_KEY` codes (e.g., `KEY_*`). Parsing is via `evdev_rs::enums::EventCode::from_str`.
- __Order of application__:
  - Global `DualRole` + active-mode `DualRole` apply first.
  - Active-mode `switch` chords are consumed so their base keys don't leak through.
  - Active-mode `remap` rules apply in file order. (Largest chord wins; `switch` beats `remap` on ties.)
- __Chord matching__ (`Remap`):
  - A rule matches if its `input` set is a subset of current keys (after DualRole transform) and not shadowed by prior rule outputs.
  - Non-modifier inputs/outputs are removed from the intermediate set to prevent subsequent rules from chaining on them.
- __Largest chord wins__:
  - If multiple `Remap` rules include the pressed key and all inputs are down, the rule with the largest `input` set is preferred.
  - When tied between a `Remap` and a `ModeSwitch` on the same chord, `ModeSwitch` takes precedence.
- __Dual role timing__:
  - Tap threshold is 200ms (`src/remapper.rs::timeval_diff`). Quick press+release emits `tap`. Longer hold emits `hold` until release.

Notes:

- Top-level `[[remap]]` are treated as `modes.default.remap`.
- `[[modes.<name>.switch]]` persistently sets the active mode on press; define as many as you want per mode (e.g., multiple ways to enter `nav`, or return to `default`).
- The mapper starts in the `"default"` mode.
- If `device_name` is omitted, it must be provided via CLI (`--device-name`).
- Use `evremap list-devices` to discover `device_name` and `phys` values.

## Rustdoc-style snippets

```rust
// src/mapping.rs
pub use evdev_rs::enums::{EV_KEY as KeyCode, EventCode, EventType};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct MappingConfig {
    pub device_name: Option<String>,
    pub phys: Option<String>,
    pub mappings: Vec<Mapping>,
}

impl MappingConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self>;
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Mapping {
    DualRole { input: KeyCode, hold: Vec<KeyCode>, tap: Vec<KeyCode>, mode: Option<String> },
    Remap { input: HashSet<KeyCode>, output: HashSet<KeyCode>, mode: Option<String> },
    ModeSwitch { input: HashSet<KeyCode>, mode: String, scope: Option<String> },
}
```

```rust
// src/mapping.rs (parsing helpers)
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid key `{0}`.  Use `evremap list-keys` to see possible keys.")]
    InvalidKey(String),
    #[error("Impossible: parsed KEY_XXX but not into an EV_KEY")]
    ImpossibleParseKey,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
struct KeyCodeWrapper { pub code: KeyCode }

impl std::convert::TryFrom<String> for KeyCodeWrapper {
    type Error = ConfigError;
    fn try_from(s: String) -> Result<KeyCodeWrapper, Self::Error>;
}

#[derive(Debug, Deserialize)]
struct DualRoleConfig { input: KeyCodeWrapper, hold: Vec<KeyCodeWrapper>, tap: Vec<KeyCodeWrapper> }

#[derive(Debug, Deserialize)]
struct RemapConfig { input: Vec<KeyCodeWrapper>, output: Vec<KeyCodeWrapper>, mode: Option<String> }

#[derive(Debug, Deserialize)]
struct ModeSwitchConfig { input: Vec<KeyCodeWrapper>, mode: String }

#[derive(Debug, Deserialize)]
struct ModeSection {
    dual_role: Vec<DualRoleConfig>,
    remap: Vec<RemapConfig>,
    #[serde(rename = "switch")]
    switch_to: Vec<ModeSwitchConfig>,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    device_name: Option<String>,
    phys: Option<String>,
    dual_role: Vec<DualRoleConfig>,
    remap: Vec<RemapConfig>,
    mode_switch: Vec<ModeSwitchConfig>,
    modes: HashMap<String, ModeSection>,
}
