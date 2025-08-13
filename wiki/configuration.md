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

## Rustdoc-style snippets

```rust
// src/mapping.rs
pub use evdev_rs::enums::{EV_KEY as KeyCode, EventCode, EventType};
use std::collections::HashSet;
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
    DualRole { input: KeyCode, hold: Vec<KeyCode>, tap: Vec<KeyCode> },
    Remap { input: HashSet<KeyCode>, output: HashSet<KeyCode> },
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
struct RemapConfig { input: Vec<KeyCodeWrapper>, output: Vec<KeyCodeWrapper> }
