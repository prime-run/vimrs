use crate::mapping::KeyCode;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub(crate) enum KeyEventType {
    Release,
    Press,
    Repeat,
    Unknown(i32),
}

impl KeyEventType {
    pub(crate) fn from_value(value: i32) -> Self {
        match value {
            0 => KeyEventType::Release,
            1 => KeyEventType::Press,
            2 => KeyEventType::Repeat,
            _ => KeyEventType::Unknown(value),
        }
    }

    pub(crate) fn value(&self) -> i32 {
        match self {
            Self::Release => 0,
            Self::Press => 1,
            Self::Repeat => 2,
            Self::Unknown(n) => *n,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveKind {
    Remap,
    ModeSwitch,
    DualRole,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveRemap {
    pub(crate) inputs: HashSet<KeyCode>,
    pub(crate) outputs: HashSet<KeyCode>,
    pub(crate) outputs_vec: Vec<KeyCode>,
    pub(crate) kind: ActiveKind,
    pub(crate) mode: Option<String>,
}
