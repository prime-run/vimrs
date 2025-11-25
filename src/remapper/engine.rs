use crate::mapping::*;
use evdev_rs::TimeVal;
use std::collections::{HashMap, HashSet};

use super::types::*;

pub(crate) struct RemapEngine {
    pub(crate) input_state: HashMap<KeyCode, TimeVal>,
    pub(crate) mappings: Vec<Mapping>,
    pub(crate) tapping: Option<KeyCode>,
    pub(crate) output_keys: HashSet<KeyCode>,
    pub(crate) suppressed_until_released: HashSet<KeyCode>,
    pub(crate) active_remaps: Vec<ActiveRemap>,
    pub(crate) active_mode: Option<String>,
}

impl RemapEngine {
    pub(crate) fn new(mappings: Vec<Mapping>) -> Self {
        Self {
            input_state: HashMap::new(),
            output_keys: HashSet::new(),
            tapping: None,
            suppressed_until_released: HashSet::new(),
            active_remaps: Vec::new(),
            active_mode: Some("default".to_string()),
            mappings,
        }
    }

    pub(crate) fn compute_keys(&self) -> HashSet<KeyCode> {
        let mut keys: HashSet<KeyCode> = self.input_state.keys().cloned().collect();
        for s in &self.suppressed_until_released {
            keys.remove(s);
        }

        for map in &self.mappings {
            if let Mapping::DualRole { input, hold, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && keys.contains(input) {
                    keys.remove(input);
                    for h in hold {
                        keys.insert(*h);
                    }
                }
            }
        }

        for ar in &self.active_remaps {
            if ar.kind == ActiveKind::Remap {
                let mode_ok = match (ar.mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok {
                    for i in &ar.inputs {
                        keys.remove(i);
                    }
                    for o in &ar.outputs {
                        keys.insert(*o);
                    }
                }
            }
        }

        keys
    }

    pub(crate) fn lookup_dual_role_index(&self, code: KeyCode) -> Option<usize> {
        for (idx, map) in self.mappings.iter().enumerate() {
            if let Mapping::DualRole { input, mode, .. } = map {
                let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                    (None, _) => true,
                    (Some(_m), None) => false,
                    (Some(m), Some(active)) => m == active,
                };
                if mode_ok && *input == code {
                    return Some(idx);
                }
            }
        }
        None
    }

    pub(crate) fn lookup_mapping_index(&self, code: KeyCode) -> Option<usize> {
        let mut best_idx: Option<usize> = None;
        let mut best_len: usize = 0;
        let mut best_pri: u8 = 0;
        for (idx, map) in self.mappings.iter().enumerate() {
            match map {
                Mapping::DualRole { input, mode, .. } => {
                    let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_m), None) => false,
                        (Some(m), Some(active)) => m == active,
                    };
                    if mode_ok && *input == code {
                        return Some(idx);
                    }
                },
                Mapping::Remap { input, mode, .. } => {
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_m), None) => false,
                        (Some(m), Some(active)) => m == active,
                    };
                    if code_matched && all_matched && mode_ok {
                        let cand_len = input.len();
                        let cand_pri = 0u8;
                        if best_idx.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_idx = Some(idx);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
                Mapping::Macro { input, mode, .. } => {
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    let mode_ok = match (mode.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_m), None) => false,
                        (Some(m), Some(active)) => m == active,
                    };
                    if code_matched && all_matched && mode_ok {
                        let cand_len = input.len();
                        let cand_pri = 0u8;
                        if best_idx.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_idx = Some(idx);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
                Mapping::ModeSwitch { input, scope, .. } => {
                    let mut code_matched = false;
                    let mut all_matched = true;
                    for i in input {
                        if *i == code {
                            code_matched = true;
                        } else if !self.input_state.contains_key(i) {
                            all_matched = false;
                            break;
                        }
                    }
                    let scope_ok = match (scope.as_ref(), self.active_mode.as_ref()) {
                        (None, _) => true,
                        (Some(_s), None) => false,
                        (Some(s), Some(active)) => s == active,
                    };
                    if scope_ok && code_matched && all_matched {
                        let cand_len = input.len();
                        let cand_pri = 1u8;
                        if best_idx.is_none()
                            || cand_len > best_len
                            || (cand_len == best_len && cand_pri > best_pri)
                        {
                            best_idx = Some(idx);
                            best_len = cand_len;
                            best_pri = cand_pri;
                        }
                    }
                },
            }
        }
        best_idx
    }

    pub(crate) fn cancel_pending_tap(&mut self) {
        self.tapping.take();
    }

    pub(crate) fn prune_suppressed_keys(&mut self) {
        self.suppressed_until_released.retain(|k| self.input_state.contains_key(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev_rs::enums::EV_KEY::*;
    use std::collections::HashSet;

    #[test]
    fn basic_remap() {
        let mappings = vec![Mapping::Remap {
            input: [KEY_A].iter().cloned().collect(),
            output: [KEY_X].iter().cloned().collect(),
            mode: None,
        }];
        let mut s = RemapEngine::new(mappings);
        s.input_state.insert(KEY_A, TimeVal::new(0, 0));
        s.active_remaps.push(ActiveRemap {
            inputs: [KEY_A].iter().cloned().collect(),
            outputs: [KEY_X].iter().cloned().collect(),
            outputs_vec: vec![KEY_X],
            kind: ActiveKind::Remap,
            mode: None,
        });

        let keys = s.compute_keys();
        let mut expected = HashSet::new();
        expected.insert(KEY_X);

        assert_eq!(keys, expected);
    }

    #[test]
    fn test_remap_edge() {
        let mappings = vec![
            Mapping::Remap {
                input: [KEY_LEFTALT, KEY_F].iter().cloned().collect(),
                output: [KEY_MINUS].iter().cloned().collect(),
                mode: Some("default".to_string()),
            },
            Mapping::Remap {
                input: [KEY_LEFTALT, KEY_LEFTBRACE].iter().cloned().collect(),
                output: [KEY_LEFTSHIFT, KEY_9].iter().cloned().collect(),
                mode: Some("default".to_string()),
            },
        ];

        let mut s = RemapEngine::new(mappings);

        s.input_state.insert(KEY_LEFTALT, TimeVal::new(0, 0));

        s.input_state.insert(KEY_F, TimeVal::new(0, 1));
        s.active_remaps.push(ActiveRemap {
            inputs: [KEY_LEFTALT, KEY_F].iter().cloned().collect(),
            outputs: [KEY_MINUS].iter().cloned().collect(),
            outputs_vec: vec![KEY_MINUS],
            kind: ActiveKind::Remap,
            mode: Some("default".to_string()),
        });

        let keys_after_f = s.compute_keys();
        let mut expected_after_f = HashSet::new();
        expected_after_f.insert(KEY_MINUS);
        assert_eq!(keys_after_f, expected_after_f);

        s.input_state.insert(KEY_LEFTBRACE, TimeVal::new(0, 2));
        s.active_remaps.push(ActiveRemap {
            inputs: [KEY_LEFTALT, KEY_LEFTBRACE].iter().cloned().collect(),
            outputs: [KEY_LEFTSHIFT, KEY_9].iter().cloned().collect(),
            outputs_vec: vec![KEY_LEFTSHIFT, KEY_9],
            kind: ActiveKind::Remap,
            mode: Some("default".to_string()),
        });

        let keys_after_leftbrace = s.compute_keys();
        let mut expected_after_leftbrace = HashSet::new();
        expected_after_leftbrace.insert(KEY_MINUS);
        expected_after_leftbrace.insert(KEY_LEFTSHIFT);
        expected_after_leftbrace.insert(KEY_9);
        assert_eq!(keys_after_leftbrace, expected_after_leftbrace);
    }

    #[test]
    fn noop_remap_suppresses_key() {
        let mappings = vec![Mapping::Remap {
            input: [KEY_A].iter().cloned().collect(),
            output: [].iter().cloned().collect(),
            mode: Some("gaming".to_string()),
        }];

        let mut s = RemapEngine::new(mappings);
        s.active_mode = Some("gaming".to_string());

        s.input_state.insert(KEY_A, TimeVal::new(0, 0));
        s.active_remaps.push(ActiveRemap {
            inputs: [KEY_A].iter().cloned().collect(),
            outputs: [].iter().cloned().collect(),
            outputs_vec: vec![],
            kind: ActiveKind::Remap,
            mode: Some("gaming".to_string()),
        });

        let keys = s.compute_keys();
        assert!(keys.is_empty(), "no-op remap should suppress KEY_A");
    }
}
