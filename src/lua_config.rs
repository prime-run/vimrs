use anyhow::{Context, Result, bail};
use evdev_rs::enums::EV_KEY as KeyCode;
use mlua::prelude::*;
use mlua::{Error as LuaError, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::rc::Rc;

use crate::mapping::{Mapping, MappingConfig};

#[derive(Default)]
struct Builder {
    device_name: Option<String>,
    phys: Option<String>,
    mappings: Vec<Mapping>,
    mode_exclusive: HashSet<String>,
}

fn expand_simple_token_to_keys(token: &str) -> Result<Vec<KeyCode>> {
    if token.len() == 1 {
        let ch = token.chars().next().unwrap();
        if ch.is_ascii_alphabetic() && ch.is_uppercase() {
            Ok(vec![KeyCode::KEY_LEFTSHIFT, parse_char_key(ch)?])
        } else {
            Ok(vec![parse_char_key(ch)?])
        }
    } else if token.starts_with("KEY_") {
        Ok(vec![parse_key_name(token)?])
    } else if let Ok(k) = parse_named_key(token) {
        Ok(vec![k])
    } else {
        Ok(vec![parse_key_name(token)?])
    }
}

impl Builder {
    fn push_dual_role(
        &mut self,
        input: KeyCode,
        hold: Vec<KeyCode>,
        tap: Vec<KeyCode>,
        mode: Option<String>,
    ) {
        self.mappings
            .push(Mapping::DualRole { input, hold, tap, mode });
    }

    fn push_remap(&mut self, input: Vec<KeyCode>, output: Vec<KeyCode>, mode: Option<String>) {
        let input: HashSet<KeyCode> = input.into_iter().collect();
        let output: HashSet<KeyCode> = output.into_iter().collect();
        self.mappings
            .push(Mapping::Remap { input, output, mode });
    }

    fn push_macro(
        &mut self,
        input: Vec<KeyCode>,
        phases: Vec<SequencePhase>,
        mode: Option<String>,
    ) {
        let seq = compile_sequence_phases(phases);
        let input: HashSet<KeyCode> = input.into_iter().collect();
        self.mappings
            .push(Mapping::Macro { input, seq, mode });
    }
}

#[derive(Debug, Clone)]
struct SequencePhase {
    hold: Vec<KeyCode>,
    tap: Vec<KeyCode>,
}

fn compile_sequence_phases(phases: Vec<SequencePhase>) -> Vec<crate::mapping::MacroOp> {
    use crate::mapping::MacroOp;
    let mut ops: Vec<MacroOp> = Vec::new();
    for phase in phases {
        for k in &phase.hold {
            ops.push(MacroOp::Press(*k));
        }
        for k in &phase.tap {
            ops.push(MacroOp::Press(*k));
            ops.push(MacroOp::Release(*k));
        }
        for k in phase.hold.iter().rev() {
            ops.push(MacroOp::Release(*k));
        }
    }
    ops
}

fn parse_input_chord(s: &str) -> Result<Vec<KeyCode>> {
    if s.trim_start().starts_with('<') && s.trim_end().ends_with('>') {
        let token = &s[1..s.len() - 1];
        let (mods, key) = parse_angle_token(token)?;
        let mut out = mods;
        out.push(key);
        Ok(out)
    } else {
        let mut out: Vec<KeyCode> = Vec::new();
        for part in s
            .split([' ', ','])
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
        {
            let keys = expand_simple_token_to_keys(part)?;
            out.extend(keys);
        }
        Ok(out)
    }
}

fn parse_sequence(s: &str) -> Result<Vec<SequencePhase>> {
    let mut phases = Vec::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'>' {
                j += 1;
            }
            if j >= bytes.len() {
                bail!("Unclosed <...> in sequence: {s}");
            }
            let token = &s[i + 1..j];
            let (mods, key) = parse_angle_token(token)?;
            phases.push(SequencePhase { hold: mods, tap: vec![key] });
            i = j + 1;
        } else if bytes[i].is_ascii_whitespace() {
            i += 1;
        } else {
            let ch = s[i..].chars().next().unwrap();
            let key = parse_char_key(ch)?;
            phases.push(SequencePhase { hold: vec![], tap: vec![key] });
            i += ch.len_utf8();
        }
    }
    Ok(phases)
}

fn parse_angle_token(token: &str) -> Result<(Vec<KeyCode>, KeyCode)> {
    let mut parts = token.split('-').collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("empty token <> parsed");
    }
    let last = parts.pop().unwrap();
    let mut mods = Vec::new();
    for m in parts {
        mods.extend(parse_modifier(m)?);
    }
    let key = if last.len() == 1 {
        let ch = last.chars().next().unwrap();

        if ch.is_ascii_alphabetic() && ch.is_uppercase() {
            mods.push(KeyCode::KEY_LEFTSHIFT);
        }
        parse_char_key(ch)?
    } else {
        parse_named_key(last)?
    };
    Ok((mods, key))
}

fn parse_modifier(m: &str) -> Result<Vec<KeyCode>> {
    match m.to_ascii_lowercase().as_str() {
        "c" | "ctrl" => Ok(vec![KeyCode::KEY_LEFTCTRL]),
        "s" | "shift" => Ok(vec![KeyCode::KEY_LEFTSHIFT]),

        "m" | "a" | "alt" => Ok(vec![KeyCode::KEY_LEFTALT]),
        "meta" | "super" => Ok(vec![KeyCode::KEY_LEFTMETA]),
        other => bail!("unknown modifier in <...>: {other}"),
    }
}

fn parse_char_key(c: char) -> Result<KeyCode> {
    use KeyCode::*;
    let kc = match c {
        'a'..='z' => {
            let up = c.to_ascii_uppercase();
            let idx = (up as u8) - b'A';
            let table = [
                KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I, KEY_J, KEY_K, KEY_L,
                KEY_M, KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R, KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X,
                KEY_Y, KEY_Z,
            ];
            table[idx as usize]
        },
        'A'..='Z' => {
            let idx = (c as u8) - b'A';
            let table = [
                KEY_A, KEY_B, KEY_C, KEY_D, KEY_E, KEY_F, KEY_G, KEY_H, KEY_I, KEY_J, KEY_K, KEY_L,
                KEY_M, KEY_N, KEY_O, KEY_P, KEY_Q, KEY_R, KEY_S, KEY_T, KEY_U, KEY_V, KEY_W, KEY_X,
                KEY_Y, KEY_Z,
            ];
            table[idx as usize]
        },
        '0'..='9' => {
            let idx = (c as u8) - b'0';
            let table = [
                KEY_0, KEY_1, KEY_2, KEY_3, KEY_4, KEY_5, KEY_6, KEY_7, KEY_8, KEY_9,
            ];
            table[idx as usize]
        },
        '[' => KEY_LEFTBRACE,
        ']' => KEY_RIGHTBRACE,
        '-' => KEY_MINUS,
        '=' => KEY_EQUAL,
        ';' => KEY_SEMICOLON,
        '\'' => KEY_APOSTROPHE,
        ',' => KEY_COMMA,
        '.' => KEY_DOT,
        '/' => KEY_SLASH,
        '\\' => KEY_BACKSLASH,
        '`' => KEY_GRAVE,
        ' ' => KEY_SPACE,
        _ => bail!("unsupported character key: {c}"),
    };
    Ok(kc)
}

fn parse_named_key(name: &str) -> Result<KeyCode> {
    use KeyCode::*;
    match name.to_ascii_lowercase().as_str() {
        "esc" => Ok(KEY_ESC),
        "tab" => Ok(KEY_TAB),
        "enter" | "cr" => Ok(KEY_ENTER),
        "space" => Ok(KEY_SPACE),
        "bs" | "backspace" => Ok(KEY_BACKSPACE),
        "left" => Ok(KEY_LEFT),
        "right" => Ok(KEY_RIGHT),
        "up" => Ok(KEY_UP),
        "down" => Ok(KEY_DOWN),
        other => parse_key_name(other),
    }
}

fn parse_key_name(name: &str) -> Result<KeyCode> {
    use evdev_rs::enums::{EventCode, EventType};
    let s = if name.starts_with("KEY_") {
        name.to_string()
    } else {
        format!("KEY_{}", name.to_ascii_uppercase())
    };
    match EventCode::from_str(&EventType::EV_KEY, &s) {
        Some(EventCode::EV_KEY(k)) => Ok(k),
        _ => bail!("unknown key name: {name}"),
    }
}

fn lua_to_any<T>(r: mlua::Result<T>) -> anyhow::Result<T> {
    r.map_err(|e| anyhow::anyhow!(e.to_string()))
}

fn to_lua_err<E: ToString>(e: E) -> LuaError {
    LuaError::RuntimeError(e.to_string())
}

pub fn from_lua_file<P: AsRef<Path>>(path: P) -> Result<MappingConfig> {
    let path = path.as_ref();
    let code = fs::read_to_string(path).context("reading lua config")?;
    let lua = Lua::new();
    let builder = Rc::new(RefCell::new(Builder::default()));

    let globals = lua.globals();
    let primemap = lua_to_any(lua.create_table())?;

    {
        let builder = Rc::clone(&builder);
        let func = lua_to_any(lua.create_function(
            move |_, (name, phys): (String, Option<String>)| -> LuaResult<()> {
                let mut b = builder.borrow_mut();
                b.device_name = Some(name);
                b.phys = phys;
                Ok(())
            },
        ))?;
        lua_to_any(primemap.set("device", func))?;
    }

    {
        let builder = Rc::clone(&builder);
        let mode_ctor =
            lua_to_any(lua.create_function(move |lua, name: String| -> LuaResult<LuaTable> {
                let mode_name = name.clone();
                let table = lua.create_table()?;

                // exclusive(flag: boolean)
                // mark/unmark this mode as exclusive
                {
                    let builder = Rc::clone(&builder);
                    let mode_name = mode_name.clone();
                    let f = lua.create_function(move |_, flag: bool| -> LuaResult<()> {
                        if flag {
                            builder
                                .borrow_mut()
                                .mode_exclusive
                                .insert(mode_name.clone());
                        } else {
                            builder
                                .borrow_mut()
                                .mode_exclusive
                                .remove(&mode_name);
                        }
                        Ok(())
                    })?;
                    table.set("exclusive", f)?;
                }

                {
                    let builder = Rc::clone(&builder);
                    let mode_name = mode_name.clone();
                    let f = lua.create_function(
                        move |_, (input, hold, tap): (String, Value, Value)| -> LuaResult<()> {
                            let input_key = parse_key_name(&input).map_err(to_lua_err)?;
                            let hold_vec = match hold {
                                Value::String(s) => {
                                    vec![parse_key_name(&s.to_str()?).map_err(to_lua_err)?]
                                },
                                Value::Table(t) => {
                                    let mut v = Vec::new();
                                    for pair in t.sequence_values::<String>() {
                                        v.push(
                                            parse_key_name(&pair.map_err(to_lua_err)?)
                                                .map_err(to_lua_err)?,
                                        );
                                    }
                                    v
                                },
                                _ => Vec::new(),
                            };
                            let tap_vec = match tap {
                                Value::String(s) => {
                                    vec![parse_key_name(&s.to_str()?).map_err(to_lua_err)?]
                                },
                                Value::Table(t) => {
                                    let mut v = Vec::new();
                                    for pair in t.sequence_values::<String>() {
                                        v.push(
                                            parse_key_name(&pair.map_err(to_lua_err)?)
                                                .map_err(to_lua_err)?,
                                        );
                                    }
                                    v
                                },
                                _ => Vec::new(),
                            };
                            builder.borrow_mut().push_dual_role(
                                input_key,
                                hold_vec,
                                tap_vec,
                                Some(mode_name.clone()),
                            );
                            Ok(())
                        },
                    )?;
                    table.set("dual_role", f)?;
                }

                {
                    let builder = Rc::clone(&builder);
                    let mode_name = mode_name.clone();
                    let f = lua.create_function(
                        move |_, (input, output): (Value, Value)| -> LuaResult<()> {
                            let input_vec = match input {
                                Value::String(s) => {
                                    parse_input_chord(&s.to_str()?).map_err(to_lua_err)?
                                },
                                Value::Table(t) => {
                                    let mut v = Vec::new();
                                    for s in t.sequence_values::<String>() {
                                        v.push(
                                            parse_key_name(&s.map_err(to_lua_err)?)
                                                .map_err(to_lua_err)?,
                                        );
                                    }
                                    v
                                },
                                _ => vec![],
                            };
                            let output_vec = match output {
                                Value::String(s) => {
                                    vec![parse_key_name(&s.to_str()?).map_err(to_lua_err)?]
                                },
                                Value::Table(t) => {
                                    let mut v = Vec::new();
                                    for s in t.sequence_values::<String>() {
                                        v.push(
                                            parse_key_name(&s.map_err(to_lua_err)?)
                                                .map_err(to_lua_err)?,
                                        );
                                    }
                                    v
                                },
                                _ => vec![],
                            };
                            builder.borrow_mut().push_remap(
                                input_vec,
                                output_vec,
                                Some(mode_name.clone()),
                            );
                            Ok(())
                        },
                    )?;
                    table.set("remap", f)?;
                }

                {
                    let builder = Rc::clone(&builder);
                    let mode_name = mode_name.clone();
                    let f = lua.create_function(
                        move |_, (input, seq): (String, String)| -> LuaResult<()> {
                            let input_vec = parse_input_chord(&input).map_err(to_lua_err)?;
                            let phases = parse_sequence(&seq).map_err(to_lua_err)?;
                            builder.borrow_mut().push_macro(
                                input_vec,
                                phases,
                                Some(mode_name.clone()),
                            );
                            Ok(())
                        },
                    )?;
                    table.set("remap_seq", f)?;
                }

                {
                    let builder = Rc::clone(&builder);
                    let mode_name = mode_name.clone();
                    let f = lua.create_function(
                        move |_, (input, to_mode): (String, String)| -> LuaResult<()> {
                            let input_vec = parse_input_chord(&input).map_err(to_lua_err)?;
                            let input_set: HashSet<KeyCode> = input_vec.into_iter().collect();
                            builder
                                .borrow_mut()
                                .mappings
                                .push(Mapping::ModeSwitch {
                                    input: input_set,
                                    mode: to_mode,
                                    scope: Some(mode_name.clone()),
                                });
                            Ok(())
                        },
                    )?;
                    table.set("switch", f)?;
                }

                Ok(table)
            }))?;
        lua_to_any(primemap.set("mode", mode_ctor))?;
    }

    {
        let builder = Rc::clone(&builder);

        let f = lua_to_any(lua.create_function(
            move |_, (mode, input, seq): (Option<String>, String, String)| -> LuaResult<()> {
                let input_vec = parse_input_chord(&input).map_err(to_lua_err)?;
                let phases = parse_sequence(&seq).map_err(to_lua_err)?;
                builder
                    .borrow_mut()
                    .push_macro(input_vec, phases, mode);
                Ok(())
            },
        ))?;
        lua_to_any(primemap.set("remap_seq", f))?;
    }

    lua_to_any(globals.set("primemap", primemap))?;

    if let Some(parent) = path.parent() {
        let basedir = parent.display().to_string();
        let package: LuaTable = lua_to_any(globals.get("package"))?;
        let old_path: String = lua_to_any(package.get("path"))?;
        let new_path = format!("{}/?.lua;{}/?/init.lua;{}", basedir, basedir, old_path);
        lua_to_any(package.set("path", new_path))?;
    }

    let name = path.to_string_lossy().to_string();
    let chunk = lua.load(&code).set_name(&name);
    lua_to_any(chunk.exec())?;

    {
        use evdev_rs::enums::EventCode;
        let mut b = builder.borrow_mut();
        if !b.mode_exclusive.is_empty() {
            let mut allowed: HashMap<String, HashSet<KeyCode>> = HashMap::new();
            for m in &b.mappings {
                match m {
                    Mapping::DualRole { input, mode, .. } => {
                        if let Some(mode) = mode {
                            allowed
                                .entry(mode.clone())
                                .or_default()
                                .insert(*input);
                        }
                    },
                    Mapping::Remap { input, mode, .. } => {
                        if let Some(mode) = mode {
                            let set = allowed.entry(mode.clone()).or_default();
                            for i in input {
                                set.insert(*i);
                            }
                        }
                    },
                    Mapping::Macro { input, mode, .. } => {
                        if let Some(mode) = mode {
                            let set = allowed.entry(mode.clone()).or_default();
                            for i in input {
                                set.insert(*i);
                            }
                        }
                    },
                    Mapping::ModeSwitch { input, scope, .. } => {
                        if let Some(mode) = scope {
                            let set = allowed.entry(mode.clone()).or_default();
                            for i in input {
                                set.insert(*i);
                            }
                        }
                    },
                }
            }
            let all_keys: Vec<KeyCode> = EventCode::EV_KEY(KeyCode::KEY_RESERVED)
                .iter()
                .filter_map(|ec| if let EventCode::EV_KEY(k) = ec { Some(k) } else { None })
                .collect();
            for mode in b.mode_exclusive.clone() {
                let allow = allowed
                    .entry(mode.clone())
                    .or_default()
                    .clone();
                for k in &all_keys {
                    if !allow.contains(k) {
                        let input: HashSet<KeyCode> = std::iter::once(*k).collect();
                        let output: HashSet<KeyCode> = HashSet::new();
                        b.mappings
                            .push(Mapping::Remap { input, output, mode: Some(mode.clone()) });
                    }
                }
            }
        }
        Ok(MappingConfig {
            device_name: b.device_name.clone(),
            phys: b.phys.clone(),
            mappings: b.mappings.clone(),
        })
    }
}
