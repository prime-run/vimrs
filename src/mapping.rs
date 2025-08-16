use anyhow::Context;
pub use evdev_rs::enums::{EV_KEY as KeyCode, EventCode, EventType};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct MappingConfig {
    pub device_name: Option<String>,
    pub phys: Option<String>,
    pub mappings: Vec<Mapping>,
}

impl MappingConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let toml_data = std::fs::read_to_string(path)
            .context(format!("reading toml from {}", path.display()))?;
        let config_file: ConfigFile =
            toml::from_str(&toml_data).context(format!("parsing toml from {}", path.display()))?;
        let mut mappings = vec![];
        for dual in config_file.dual_role {
            mappings.push(dual.into());
        }
        for remap in config_file.remap {
            mappings.push(remap.into());
        }
        for ms in config_file.mode_switch {
            mappings.push(ms.into());
        }

        // Nested modes: scope rules under each named mode
        for (mode_name, section) in config_file.modes {
            // Dual roles scoped to this mode
            for dual in section.dual_role {
                let map = Mapping::DualRole {
                    input: dual.input.into(),
                    hold: dual
                        .hold
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    tap: dual
                        .tap
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    mode: Some(mode_name.clone()),
                };
                mappings.push(map);
            }

            // Remaps scoped to this mode
            for remap in section.remap {
                let map = Mapping::Remap {
                    input: remap
                        .input
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    output: remap
                        .output
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    mode: Some(mode_name.clone()),
                };
                mappings.push(map);
            }

            // Switch chords defined under this mode; active only while in this mode
            for ms in section.switch_to {
                let map = Mapping::ModeSwitch {
                    input: ms
                        .input
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    mode: ms.mode,
                    scope: Some(mode_name.clone()),
                };
                mappings.push(map);
            }
        }
        Ok(Self { device_name: config_file.device_name, phys: config_file.phys, mappings })
    }
}

// #[derive(Debug, Clone, Eq, PartialEq)]
// pub enum Mode {
//     Visual,
//     Normal,
//     Insert,
//     VisualLine,
// }

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Mapping {
    DualRole {
        input: KeyCode,
        hold: Vec<KeyCode>,
        tap: Vec<KeyCode>,
        mode: Option<String>,
        // mode: Mode,
    },
    Remap {
        input: HashSet<KeyCode>,
        output: HashSet<KeyCode>,
        mode: Option<String>,
        // mode: Mode,
    },
    ModeSwitch {
        input: HashSet<KeyCode>,
        mode: String,
        scope: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
struct KeyCodeWrapper {
    pub code: KeyCode,
}

impl From<KeyCodeWrapper> for KeyCode {
    fn from(val: KeyCodeWrapper) -> Self {
        val.code
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Invalid key `{0}`.  Use `evremap list-keys` to see possible keys.")]
    InvalidKey(String),
    #[error("Impossible: parsed KEY_XXX but not into an EV_KEY")]
    ImpossibleParseKey,
}

impl std::convert::TryFrom<String> for KeyCodeWrapper {
    type Error = ConfigError;

    fn try_from(s: String) -> Result<KeyCodeWrapper, Self::Error> {
        match EventCode::from_str(&EventType::EV_KEY, &s) {
            Some(code) => match code {
                EventCode::EV_KEY(code) => Ok(KeyCodeWrapper { code }),
                _ => Err(ConfigError::ImpossibleParseKey),
            },
            None => Err(ConfigError::InvalidKey(s)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DualRoleConfig {
    input: KeyCodeWrapper,
    hold: Vec<KeyCodeWrapper>,
    tap: Vec<KeyCodeWrapper>,
}

impl From<DualRoleConfig> for Mapping {
    fn from(val: DualRoleConfig) -> Self {
        Mapping::DualRole {
            input: val.input.into(),
            hold: val
                .hold
                .into_iter()
                .map(Into::into)
                .collect(),
            tap: val
                .tap
                .into_iter()
                .map(Into::into)
                .collect(),
            mode: None,
            // mode: Mode::Insert,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RemapConfig {
    input: Vec<KeyCodeWrapper>,
    output: Vec<KeyCodeWrapper>,
    #[serde(default)]
    mode: Option<String>,
}

impl From<RemapConfig> for Mapping {
    fn from(val: RemapConfig) -> Self {
        Mapping::Remap {
            input: val
                .input
                .into_iter()
                .map(Into::into)
                .collect(),
            output: val
                .output
                .into_iter()
                .map(Into::into)
                .collect(),
            // NOTE: If no mode is specified, treat it as the implicit "default" mode.
            mode: Some(
                val.mode
                    .unwrap_or_else(|| "default".to_string()),
            ),
            // mode: Mode::Insert,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModeSwitchConfig {
    input: Vec<KeyCodeWrapper>,
    mode: String,
}

impl From<ModeSwitchConfig> for Mapping {
    fn from(val: ModeSwitchConfig) -> Self {
        Mapping::ModeSwitch {
            input: val
                .input
                .into_iter()
                .map(Into::into)
                .collect(),
            mode: val.mode,
            scope: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModeSection {
    #[serde(default)]
    dual_role: Vec<DualRoleConfig>,
    #[serde(default)]
    remap: Vec<RemapConfig>,
    #[serde(default, rename = "switch")]
    switch_to: Vec<ModeSwitchConfig>,
}

#[derive(Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    device_name: Option<String>,

    #[serde(default)]
    phys: Option<String>,

    #[serde(default)]
    dual_role: Vec<DualRoleConfig>,

    #[serde(default)]
    remap: Vec<RemapConfig>,

    #[serde(default)]
    mode_switch: Vec<ModeSwitchConfig>,

    #[serde(default)]
    modes: HashMap<String, ModeSection>,
}
